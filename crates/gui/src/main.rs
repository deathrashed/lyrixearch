// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod accounts;
mod background;
mod browser;
#[cfg(target_os = "linux")]
mod linux_panel;
mod player;
#[cfg(target_os = "macos")]
mod shelf;
mod tray;

use anyhow::Context as _;
use lyrixearch_core::providers::{self, ProviderInfo};
use lyrixearch_core::{pending, scan, tags, util, Config, Engine, Event, Lyrics, SidecarEmbed, Track};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::webview::WebviewBuilder;
use tauri::window::WindowBuilder;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, State, WebviewUrl, WindowEvent};

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Status {
    Idle,
    Queued,
    Searching,
    Found,
    Notfound,
    Written,
    Error,
}

struct Entry {
    id: u64,
    track: Track,
    status: Status,
    provider: Option<String>,
    found: Option<Lyrics>,
    embedded: Option<String>,
    note: Option<String>,
    /// The last search of this track, source by source (Info → search trail).
    trail: Vec<Attempt>,
}

/// One source in a track's last search.
#[derive(Serialize, Clone)]
struct Attempt {
    provider: String,
    /// "searching" while it runs; then "found", "no match", "skipped: …", a timeout or the error
    result: String,
    millis: u64,
}

struct Inner {
    cfg: Config,
    engine: CmdResult<Arc<Engine>>,
    settings_write_error: Option<String>,
    startup_notice: Option<String>,
    entries: Vec<Entry>,
    next_id: u64,
    cancel: Arc<AtomicBool>,
    browser_url: Option<String>,
    /// Cover thumbnails by album folder, so a 12-track album decodes its art once.
    covers: HashMap<String, Option<String>>,
    /// Paths handed to the app by the OS (dropped on the icon, "Open with", command line).
    pending_open: Vec<String>,
    zoom: f64,
    /// Load web pages with a phone user agent so they lay out for the narrow panel.
    browser_mobile: bool,
    /// Where the UI last asked the browser panel to sit, so it can be put back after a window resize.
    browser_rect: Option<Rect>,
    /// Source waiting for a bot check to be passed (its penalty is cleared afterwards).
    check_provider: Option<String>,
}

/// Cookies from bot checks passed in the verification window, shared with every engine rebuild.
fn jar() -> Arc<lyrixearch_core::CookieJar> {
    static JAR: std::sync::OnceLock<Arc<lyrixearch_core::CookieJar>> = std::sync::OnceLock::new();
    JAR.get_or_init(Default::default).clone()
}

const MOBILE_UA: &str =
    "Mozilla/5.0 (iPhone; CPU iPhone OS 17_5 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.5 Mobile/15E148 Safari/604.1";

struct AppState(Mutex<Inner>);

#[derive(Serialize, Clone)]
struct TrackDto {
    id: u64,
    path: String,
    name: String,
    folder: String,
    artist: String,
    title: String,
    album: Option<String>,
    album_artist: Option<String>,
    track_number: Option<u32>,
    format: String,
    duration: Option<u32>,
    has_lyrics: bool,
    status: Status,
    provider: Option<String>,
    score: Option<f32>,
    synced: bool,
    note: Option<String>,
}

fn dto(e: &Entry) -> TrackDto {
    TrackDto {
        id: e.id,
        path: e.track.path.to_string_lossy().into_owned(),
        name: e.track.display_name(),
        folder: e.track.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
        artist: e.track.query.artist.clone(),
        title: e.track.query.title.clone(),
        album: e.track.query.album.clone(),
        album_artist: e.track.query.album_artist.clone(),
        track_number: e.track.query.track_number,
        format: e.track.format.clone(),
        duration: e.track.query.duration_secs,
        has_lyrics: e.track.has_lyrics,
        status: e.status,
        provider: e.provider.clone(),
        score: e.found.as_ref().map(|l| l.score),
        synced: e.found.as_ref().is_some_and(|l| l.synced.is_some()),
        note: e.note.clone(),
    }
}

type CmdResult<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    format!("{e:#}")
}

#[derive(Serialize, Clone)]
struct Stats {
    total: usize,
    with_lyrics: usize,
    missing: usize,
}

#[derive(Serialize)]
struct TrackList {
    tracks: Vec<TrackDto>,
    stats: Stats,
    skipped: usize,
}

fn list(inner: &Inner, skipped: usize) -> TrackList {
    TrackList { tracks: inner.entries.iter().map(dto).collect(), stats: stats(inner), skipped }
}

fn stats(inner: &Inner) -> Stats {
    let with_lyrics = inner.entries.iter().filter(|e| e.track.has_lyrics).count();
    let total = inner.entries.len();
    Stats { total, with_lyrics, missing: total - with_lyrics }
}

fn emit_track(app: &AppHandle, state: &State<'_, AppState>, id: u64) -> Option<TrackDto> {
    let track = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto)?;
    let _ = app.emit("track", &track);
    Some(track)
}

fn update_pending(app: &AppHandle, update: impl FnOnce(&mut pending::Pending)) {
    if let Err(e) = pending::update(update) {
        let _ = app.emit("operation-warning", format!("Could not update pending lyrics: {e:#}"));
    }
}

fn forget_pending(app: &AppHandle, paths: &[PathBuf]) {
    if !paths.is_empty() {
        update_pending(app, |map| {
            for path in paths {
                map.remove(path);
            }
        });
    }
}

// ───────────────────────────── library ─────────────────────────────

#[tauri::command]
fn list_tracks(state: State<'_, AppState>) -> TrackList {
    list(&state.0.lock().unwrap(), 0)
}

/// Scan files/folders (recursively) and add the audio files to the working list.
#[tauri::command]
async fn add_paths(app: AppHandle, state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<TrackList> {
    let known: HashSet<PathBuf> = state.0.lock().unwrap().entries.iter().map(|e| e.track.path.clone()).collect();
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    // Tag reading touches every file: keep it off the async threads.
    let (loaded, skipped) = tauri::async_runtime::spawn_blocking(move || {
        let mut loaded = Vec::new();
        let mut skipped = 0usize;
        for f in scan::collect_audio(&paths, true).into_iter().filter(|f| !known.contains(f)) {
            match tags::read_track(&f) {
                Ok(t) => {
                    let embedded = if t.has_lyrics { tags::read_lyrics(&t.path) } else { None };
                    loaded.push((t, embedded));
                }
                Err(_) => skipped += 1,
            }
        }
        (loaded, skipped)
    })
    .await
    .map_err(err)?;

    let mut inner = state.0.lock().unwrap();
    let pending = pending::load().unwrap_or_else(|e| {
        let _ = app.emit("operation-warning", format!("Could not load pending lyrics: {e:#}"));
        pending::Pending::new()
    });
    for (track, embedded) in loaded {
        let id = inner.next_id;
        inner.next_id += 1;
        let found = pending.get(&util::pending_key(&track.path)).cloned();
        let provider = found.as_ref().map(|lyrics| lyrics.source.clone());
        let status = if found.is_some() { Status::Found } else { Status::Idle };
        inner.entries.push(Entry { id, track, status, provider, found, embedded, note: None, trail: Vec::new() });
    }
    Ok(list(&inner, skipped))
}

#[tauri::command]
fn remove_tracks(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>) -> TrackList {
    let mut inner = state.0.lock().unwrap();
    let ids: HashSet<u64> = ids.into_iter().collect();
    let discarded = inner.entries.iter().filter(|e| ids.contains(&e.id) && e.found.is_some()).map(|e| util::pending_key(&e.track.path)).collect::<Vec<_>>();
    forget_pending(&app, &discarded);
    inner.entries.retain(|e| !ids.contains(&e.id));
    list(&inner, 0)
}

#[tauri::command]
fn clear_tracks(app: AppHandle, state: State<'_, AppState>) -> TrackList {
    let mut inner = state.0.lock().unwrap();
    let discarded = inner.entries.iter().filter(|e| e.found.is_some()).map(|e| util::pending_key(&e.track.path)).collect::<Vec<_>>();
    forget_pending(&app, &discarded);
    inner.entries.clear();
    list(&inner, 0)
}

// ───────────────────────────── searching ─────────────────────────────

#[derive(Serialize, Clone)]
struct Progress {
    done: usize,
    total: usize,
    found: usize,
    written: usize,
    missed: usize,
    running: bool,
}

/// Search the given tracks (or every track still missing lyrics when `ids` is empty). `replace`
/// decides whether tracks that already have lyrics are searched too. With
/// `auto_write`, every hit is embedded immediately — the "find and embed everything" button.
#[tauri::command]
async fn search(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>, auto_write: bool, replace: Option<bool>) -> CmdResult<usize> {
    let (engine, cancel, work) = {
        let mut inner = state.0.lock().unwrap();
        let engine = inner.engine.clone()?;
        let overwrite = inner.cfg.overwrite;
        let want: Option<HashSet<u64>> = if ids.is_empty() { None } else { Some(ids.into_iter().collect()) };
        let mut work = Vec::new();
        for e in inner.entries.iter_mut() {
            let picked = want.as_ref().is_none_or(|w| w.contains(&e.id));
            let busy = matches!(e.status, Status::Queued | Status::Searching);
            // `replace: Some(false)` skips tracks that already have lyrics even when they were named explicitly.
            let allowed = match replace {
                Some(r) => r || !e.track.has_lyrics,
                None => overwrite || !e.track.has_lyrics || want.is_some(),
            };
            if picked && !busy && allowed {
                e.status = Status::Queued;
                e.trail.clear();
                e.note = None;
                work.push((e.id as usize, e.track.query.clone()));
            }
        }
        inner.cancel.store(false, Ordering::Relaxed);
        (engine, inner.cancel.clone(), work)
    };
    let total = work.len();
    if total == 0 {
        return Ok(0);
    }
    for e in state.0.lock().unwrap().entries.iter().filter(|e| e.status == Status::Queued) {
        let _ = app.emit("track", dto(e));
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let eng = engine.clone();
    tauri::async_runtime::spawn(async move { eng.run(work, tx, cancel).await });

    let app2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let state = app2.state::<AppState>();
        let (keep_awake, notify_done) = {
            let inner = state.0.lock().unwrap();
            (inner.cfg.keep_awake, inner.cfg.notify_done)
        };
        // held until this task ends, whichever way the fetch finishes
        let _awake = keep_awake.then(background::Awake::start);
        let mut prog = Progress { done: 0, total, found: 0, written: 0, missed: 0, running: true };
        let mut failed = 0usize;
        let cancel_flag = state.0.lock().unwrap().cancel.clone();
        tray::set(&app2, tray::TrayState::Fetching, format!("fetching 0 of {total}"));
        let _ = app2.emit("session-log", format!("Started search for {total} track(s)"));
        let _ = app2.emit("progress", prog.clone());
        while let Some(ev) = rx.recv().await {
            if let Event::Challenge { provider, url } = &ev {
                let _ = app2.emit("challenge", serde_json::json!({ "provider": provider, "url": url }));
                continue;
            }
            let id = match &ev {
                Event::Trying { idx, .. }
                | Event::Skipped { idx, .. }
                | Event::Tried { idx, .. }
                | Event::Found { idx, .. }
                | Event::NotFound { idx, .. }
                | Event::Failed { idx, .. }
                | Event::Cancelled { idx } => *idx as u64,
                Event::Challenge { .. } => continue,
            };
            match ev {
                Event::Trying { provider, .. } => {
                    set(&state, id, |e| {
                        e.status = Status::Searching;
                        e.trail.push(Attempt { provider: provider.clone(), result: "searching".into(), millis: 0 });
                        e.provider = Some(provider);
                    });
                }
                Event::Skipped { provider, reason, .. } => {
                    set(&state, id, |e| e.trail.push(Attempt { provider, result: format!("skipped: {reason}"), millis: 0 }));
                    continue;
                }
                Event::Tried { provider, result, millis, .. } => {
                    // for the Activity panel's per-source chart
                    let provider_name = provider.clone();
                    let result_kind = if result == "found" {
                        "found"
                    } else if result == "no match" || result.starts_with("only unsynced") {
                        "miss"
                    } else if result.contains("timed out") {
                        "timeout"
                    } else {
                        "fail"
                    };
                    set(&state, id, |e| {
                        if let Some(a) = e.trail.iter_mut().rev().find(|a| a.provider == provider && a.result == "searching") {
                            a.result = result;
                            a.millis = millis;
                        }
                    });
                    let _ = app2.emit("trail", serde_json::json!({ "id": id, "provider": provider_name, "result": result_kind, "millis": millis }));
                    continue;
                }
                Event::Challenge { .. } => continue,
                Event::Cancelled { .. } => {
                    prog.done += 1;
                    set(&state, id, |e| e.status = Status::Idle);
                }
                Event::Failed { notes, .. } => {
                    prog.done += 1;
                    failed += 1;
                    set(&state, id, |e| {
                        e.status = Status::Error;
                        e.note = Some(format!("no source answered, search again later\n{}", notes.join("\n")));
                    });
                }
                Event::NotFound { notes, .. } => {
                    prog.done += 1;
                    prog.missed += 1;
                    set(&state, id, |e| {
                        e.status = Status::Notfound;
                        e.note = Some(notes.join("\n"));
                    });
                }
                Event::Found { lyrics, .. } => {
                    prog.done += 1;
                    prog.found += 1;
                    let lyrics = *lyrics;
                    let track = {
                        let mut inner = state.0.lock().unwrap();
                        inner.entries.iter_mut().find(|e| e.id == id).map(|e| {
                            let key = util::pending_key(&e.track.path);
                            update_pending(&app2, |map| {
                                map.insert(key, lyrics.clone());
                            });
                            e.found = Some(lyrics.clone());
                            e.track.clone()
                        })
                    };
                    let mut applied = None;
                    // a transcription waits for the person to check it
                    if auto_write && !lyrics.review {
                        if let Some(t) = track {
                            let _ = app2.emit("session-log", format!("Found {} via {}", t.display_name(), lyrics.source));
                            let l = lyrics.clone();
                            let engine = engine.clone();
                            let r = tauri::async_runtime::spawn_blocking(move || apply_result(&engine, &t, &l)).await;
                            applied = Some(r.map_err(err).flatten());
                        }
                    }
                    if let Some(Ok(_)) = &applied {
                        prog.written += 1;
                    }
                    set(&state, id, |e| {
                        e.provider = Some(lyrics.source.clone());
                        match &applied {
                            Some(Ok(tag_written)) => {
                                forget_pending(&app2, &[util::pending_key(&e.track.path)]);
                                e.status = Status::Written;
                                if *tag_written {
                                    e.track.has_lyrics = true;
                                    e.embedded = lyrics.embed_text();
                                }
                            }
                            Some(Err(m)) => {
                                e.status = Status::Error;
                                e.note = Some(m.clone());
                            }
                            None => e.status = Status::Found,
                        }
                        e.found = Some(lyrics);
                    });
                }
            }
            emit_track(&app2, &state, id);
            let _ = app2.emit("progress", prog.clone());
            tray::set(&app2, tray::TrayState::Fetching, format!("fetching {} of {total} · {} found", prog.done, prog.found));
        }
        prog.running = false;
        let summary = format!("{} found, {} not found", prog.found, prog.missed);
        let stopped = cancel_flag.load(Ordering::Relaxed) && prog.done < total;
        if stopped {
            tray::set(&app2, tray::TrayState::Stopped, format!("stopped at {} of {total} · {summary}", prog.done));
        } else if failed > 0 {
            tray::set(&app2, tray::TrayState::Error, format!("{failed} could not be searched · {summary}"));
        } else {
            tray::set(&app2, tray::TrayState::Complete, format!("done · {summary}"));
        }
        // a stop is the person's own doing, so it needs no notification
        if notify_done && !stopped && background::in_background(&app2) {
            let title = if failed > 0 { "Fetch finished with problems" } else { "Fetch finished" };
            let mut body = format!("{} of {total} tracks: {summary}", prog.done);
            if failed > 0 {
                body.push_str(&format!(", {failed} could not be searched"));
            }
            background::notify(&app2, title, &body);
        }
        let _ = app2.emit(
            "session-log",
            format!("Search finished: {}/{} completed, {} found, {} written, {} not found", prog.done, total, prog.found, prog.written, prog.missed),
        );
        let _ = app2.emit("progress", prog);
        let _ = app2.emit("stats", stats(&state.0.lock().unwrap()));
    });
    Ok(total)
}

fn set(state: &State<'_, AppState>, id: u64, f: impl FnOnce(&mut Entry)) {
    if let Some(e) = state.0.lock().unwrap().entries.iter_mut().find(|e| e.id == id) {
        f(e);
    }
}

#[tauri::command]
fn cancel_search(state: State<'_, AppState>) {
    state.0.lock().unwrap().cancel.store(true, Ordering::Relaxed);
}

// ───────────────────────────── results & editing ─────────────────────────────

#[derive(Serialize)]
struct LyricsView {
    found: Option<FoundView>,
    embedded: Option<String>,
    /// Timed lyrics stored in the file (ID3 SYLT / Vorbis SYNCEDLYRICS), for the Info timeline.
    embedded_synced: Option<String>,
    trail: Vec<Attempt>,
    /// File size in bytes.
    size: Option<u64>,
}

#[derive(Serialize)]
struct FoundView {
    source: String,
    url: Option<String>,
    matched_artist: String,
    matched_title: String,
    score: f32,
    plain: Option<String>,
    synced: Option<String>,
}

#[tauri::command]
fn get_lyrics(state: State<'_, AppState>, id: u64) -> Option<LyricsView> {
    let path = state.0.lock().unwrap().entries.iter().find(|e| e.id == id)?.track.path.clone();
    // file reads happen outside the lock
    let embedded_synced = tags::read_synced(&path);
    let size = std::fs::metadata(&path).ok().map(|m| m.len());
    let inner = state.0.lock().unwrap();
    let e = inner.entries.iter().find(|e| e.id == id)?;
    Some(LyricsView {
        embedded_synced,
        trail: e.trail.clone(),
        size,
        found: e.found.as_ref().map(|l| FoundView {
            source: l.source.clone(),
            url: l.url.clone(),
            matched_artist: l.matched_artist.clone(),
            matched_title: l.matched_title.clone(),
            score: l.score,
            plain: l.embed_text(),
            synced: l.synced.clone(),
        }),
        embedded: e.embedded.clone(),
    })
}

fn apply_result(engine: &Engine, track: &Track, lyrics: &Lyrics) -> CmdResult<bool> {
    let applied = engine.apply(track, lyrics).map_err(err)?;
    if !applied.tag_written && applied.sidecar.is_none() {
        return Err("No lyrics written. Check the tag, sidecar and replace settings.".into());
    }
    Ok(applied.tag_written)
}

/// Write found lyrics for these tracks to disk (tags and/or sidecars, per settings).
#[tauri::command]
async fn apply_found(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>) -> CmdResult<usize> {
    let ids: HashSet<u64> = ids.into_iter().collect();
    let (engine, jobs) = {
        let inner = state.0.lock().unwrap();
        let jobs: Vec<(u64, Track, Lyrics)> = inner
            .entries
            .iter()
            .filter(|e| ids.is_empty() || ids.contains(&e.id))
            .filter(|e| matches!(e.status, Status::Found | Status::Error))
            .filter_map(|e| e.found.clone().map(|l| (e.id, e.track.clone(), l)))
            .collect();
        (inner.engine.clone()?, jobs)
    };
    let mut ok = 0;
    for (id, track, lyrics) in jobs {
        let eng = engine.clone();
        let l = lyrics.clone();
        let r = tauri::async_runtime::spawn_blocking(move || apply_result(&eng, &track, &l)).await.map_err(err)?;
        set(&state, id, |e| match &r {
            Ok(tag_written) => {
                forget_pending(&app, &[util::pending_key(&e.track.path)]);
                e.status = Status::Written;
                e.note = None;
                if *tag_written {
                    e.track.has_lyrics = true;
                    e.embedded = lyrics.embed_text();
                }
            }
            Err(m) => {
                e.status = Status::Error;
                e.note = Some(format!("{m:#}"));
            }
        });
        if r.is_ok() {
            ok += 1;
        }
        emit_track(&app, &state, id);
    }
    let _ = app.emit("stats", stats(&state.0.lock().unwrap()));
    Ok(ok)
}

/// Save the editor contents, preserving timing when saving an unchanged found result.
#[tauri::command]
async fn save_lyrics(app: AppHandle, state: State<'_, AppState>, id: u64, text: String, from_found: Option<bool>) -> CmdResult<TrackDto> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("lyrics are empty — use “Remove lyrics” to clear them".into());
    }
    let (path, keep, synced) = {
        let inner = state.0.lock().unwrap();
        let e = inner.entries.iter().find(|e| e.id == id).context("unknown track").map_err(err)?;
        let synced = e
            .found
            .as_ref()
            .filter(|lyrics| from_found == Some(true) && lyrics.embed_text().as_deref() == Some(text.as_str()))
            .and_then(|lyrics| lyrics.synced.as_ref())
            .filter(|text| lyrixearch_core::lrc::looks_synced(text))
            .cloned();
        (e.track.path.clone(), inner.cfg.keep_mtime, synced)
    };
    let t = text.clone();
    tauri::async_runtime::spawn_blocking(move || match synced {
        Some(synced) => tags::write_lyrics_with(&path, &t, Some(&synced), keep),
        None => tags::write_lyrics(&path, &t, keep),
    })
    .await
    .map_err(err)?
    .map_err(err)?;
    set(&state, id, |e| {
        e.track.has_lyrics = true;
        e.embedded = Some(text);
        e.status = Status::Written;
        if e.found.take().is_some() {
            forget_pending(&app, &[util::pending_key(&e.track.path)]);
        }
        e.note = None;
    });
    let d = emit_track(&app, &state, id).context("unknown track").map_err(err)?;
    let _ = app.emit("stats", stats(&state.0.lock().unwrap()));
    Ok(d)
}

#[tauri::command]
async fn delete_lyrics(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>) -> CmdResult<usize> {
    let (jobs, keep) = {
        let inner = state.0.lock().unwrap();
        let ids: HashSet<u64> = ids.into_iter().collect();
        (
            inner.entries.iter().filter(|e| ids.contains(&e.id) && e.track.has_lyrics).map(|e| (e.id, e.track.path.clone())).collect::<Vec<_>>(),
            inner.cfg.keep_mtime,
        )
    };
    let mut n = 0;
    for (id, path) in jobs {
        let r = tauri::async_runtime::spawn_blocking(move || tags::remove_lyrics(&path, keep)).await.map_err(err)?;
        match r {
            Ok(()) => {
                n += 1;
                set(&state, id, |e| {
                    e.track.has_lyrics = false;
                    e.embedded = None;
                    e.status = Status::Idle;
                });
            }
            Err(m) => set(&state, id, |e| {
                e.status = Status::Error;
                e.note = Some(format!("{m:#}"));
            }),
        }
        emit_track(&app, &state, id);
    }
    let _ = app.emit("stats", stats(&state.0.lock().unwrap()));
    Ok(n)
}

// ───────────────────────────── settings & plugins ─────────────────────────────

#[derive(Serialize, Clone)]
struct Settings {
    config: Config,
    providers: Vec<ProviderInfo>,
    engines: Vec<&'static str>,
    plugins_dir: String,
    config_path: String,
    startup_notice: Option<String>,
    /// False where web pages cannot be shown inside the main window (see [`EMBEDDED_BROWSER`]).
    embedded_browser: bool,
}

/// Whether a web page can be shown inside the main window. Everywhere now: on Linux the page is
/// placed by [`linux_panel`], since Tauri itself stacks child webviews there.
const EMBEDDED_BROWSER: bool = true;

/// Linux: position the page in the overlay (Tauri's own position and size calls do nothing there).
#[cfg(target_os = "linux")]
fn place_page(wv: &tauri::Webview, rect: &Rect) {
    let z = rect.zoom();
    linux_panel::place(wv, linux_panel::Place { x: rect.x * z, y: rect.y * z, w: rect.w * z, h: rect.h * z });
}

fn settings_of(cfg: &Config) -> Settings {
    Settings {
        config: cfg.clone(),
        providers: providers::catalog(&Config::plugins_dir()),
        engines: util::SEARCH_ENGINES.to_vec(),
        plugins_dir: Config::plugins_dir().display().to_string(),
        config_path: Config::path().map(|p| p.display().to_string()).unwrap_or_default(),
        startup_notice: None,
        embedded_browser: EMBEDDED_BROWSER,
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    let mut inner = state.0.lock().unwrap();
    Settings { startup_notice: inner.startup_notice.take(), ..settings_of(&inner.cfg) }
}

#[tauri::command]
fn set_settings(app: AppHandle, state: State<'_, AppState>, config: Config) -> CmdResult<Settings> {
    let mut inner = state.0.lock().unwrap();
    if let Some(error) = &inner.settings_write_error {
        return Err(error.clone());
    }
    let engine = Engine::with_cookies(config.clone(), jar()).map_err(err)?;
    config.save().map_err(err)?;
    inner.cfg = config;
    inner.engine = Ok(engine);
    let out = settings_of(&inner.cfg);
    // both windows edit settings: each keeps the other's latest changes
    let _ = app.emit("settings-changed", &out);
    Ok(out)
}

#[tauri::command]
fn install_plugin(app: AppHandle, state: State<'_, AppState>, dir: String) -> CmdResult<Settings> {
    use providers::plugin::ExternalProvider;
    let src = PathBuf::from(&dir);
    if let Some(error) = &state.0.lock().unwrap().settings_write_error {
        return Err(error.clone());
    }
    // copies the folder and runs the plugin's build step (if any) without holding the state lock
    let loaded = ExternalProvider::install(&src, &Config::plugins_dir()).map_err(err)?;
    let mut inner = state.0.lock().unwrap();
    let id = loaded.manifest().id.clone();
    let mut cfg = inner.cfg.clone();
    if !cfg.providers.contains(&id) {
        cfg.providers.push(id);
    }
    let engine = Engine::with_cookies(cfg.clone(), jar()).map_err(err)?;
    cfg.save().map_err(err)?;
    inner.cfg = cfg;
    inner.engine = Ok(engine);
    let out = settings_of(&inner.cfg);
    let _ = app.emit("settings-changed", &out);
    Ok(out)
}

#[derive(Serialize)]
struct ProviderHealth {
    id: String,
    ok: bool,
    detail: String,
    millis: u128,
}

#[tauri::command]
async fn test_providers(state: State<'_, AppState>) -> CmdResult<Vec<ProviderHealth>> {
    let mut cfg = state.0.lock().unwrap().cfg.clone();
    cfg.providers = providers::catalog(&Config::plugins_dir()).into_iter().filter(|p| p.id != "local_files").map(|p| p.id).collect();
    let engine = Engine::with_cookies(cfg, jar()).map_err(err)?;
    Ok(engine
        .test_providers()
        .await
        .into_iter()
        .map(|(id, r, took)| {
            let (ok, detail) = match r {
                Ok(true) => (true, "ok".to_string()),
                Ok(false) => (false, "reachable, but no result for the test track".to_string()),
                Err(e) => (false, e),
            };
            ProviderHealth { id, ok, detail, millis: took.as_millis() }
        })
        .collect())
}

// ───────────────────────────── in-window browser ─────────────────────────────

#[derive(serde::Deserialize, Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    /// Height of the UI's own viewport, to line the page up with it (see [`page_position`]).
    #[serde(default)]
    vh: Option<f64>,
    /// The UI's zoom (interface scale): its CSS pixels times this are window pixels.
    #[serde(default)]
    scale: Option<f64>,
}

impl Rect {
    fn zoom(&self) -> f64 {
        self.scale.filter(|z| *z > 0.0).unwrap_or(1.0)
    }
}

/// Where the page goes, in the window's coordinates. On macOS the window's height includes the title bar
/// while the UI is laid out below it, so a slot measured in the UI would land one title bar too high; the
/// difference between the two heights is that offset (0 where they agree).
fn page_position(window: &tauri::Window, rect: &Rect) -> LogicalPosition<f64> {
    let offset = match (rect.vh, window.inner_size(), window.scale_factor()) {
        (Some(vh), Ok(size), Ok(scale)) => (size.to_logical::<f64>(scale).height - vh * rect.zoom()).max(0.0),
        _ => 0.0,
    };
    LogicalPosition::new(rect.x * rect.zoom(), rect.y * rect.zoom() + offset)
}

fn parse_url(u: &str) -> CmdResult<tauri::Url> {
    let url: tauri::Url = u.parse().map_err(err)?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("only http(s) pages can be shown".into());
    }
    Ok(url)
}

/// Show (creating on first use) the embedded browser at `rect`, optionally navigating.
#[tauri::command]
fn browser_show(app: AppHandle, state: State<'_, AppState>, url: Option<String>, rect: Rect) -> CmdResult<()> {
    if !EMBEDDED_BROWSER {
        return Err("Pages cannot be shown inside the window on this system; use Browser to open them in your browser".into());
    }
    let window = app.get_window("main").context("main window").map_err(err)?;
    let pos = page_position(&window, &rect);
    let size = LogicalSize::new((rect.w * rect.zoom()).max(1.0), (rect.h * rect.zoom()).max(1.0));
    state.0.lock().unwrap().browser_rect = Some(rect);
    if std::env::var_os("LYRIXEARCH_BROWSER_DEBUG").is_some() {
        let ui = app.get_webview("ui");
        eprintln!(
            "browser_show rect=({}, {}, {}, {}) ui_pos={:?} ui_size={:?} inner={:?} outer_pos={:?} inner_pos={:?} browser_pos={:?}",
            rect.x,
            rect.y,
            rect.w,
            rect.h,
            ui.as_ref().and_then(|w| w.position().ok()),
            ui.as_ref().and_then(|w| w.size().ok()),
            window.inner_size().ok(),
            window.outer_position().ok(),
            window.inner_position().ok(),
            app.get_webview("browser").and_then(|w| w.position().ok())
        );
    }
    if let Some(wv) = app.get_webview("browser") {
        wv.set_position(pos).map_err(err)?;
        wv.set_size(size).map_err(err)?;
        #[cfg(target_os = "linux")]
        place_page(&wv, &rect);
        wv.show().map_err(err)?;
        if let Some(u) = url {
            let mut inner = state.0.lock().unwrap();
            if inner.browser_url.as_deref() != Some(u.as_str()) {
                wv.navigate(parse_url(&u)?).map_err(err)?;
                inner.browser_url = Some(u);
            }
        }
        return Ok(());
    }
    let u = url.unwrap_or_else(|| "https://www.google.com/".into());
    let (mobile, zoom) = {
        let inner = state.0.lock().unwrap();
        (inner.browser_mobile, inner.zoom)
    };
    // remember the last text the person selected: clicking "Use selection" moves focus to the UI,
    // and WebKit clears the page's selection when that happens
    let mut builder = WebviewBuilder::new("browser", WebviewUrl::External(parse_url(&u)?)).initialization_script(
        "document.addEventListener('selectionchange', () => { const s = String(window.getSelection()); if (s.trim()) window.__lxSel = s; });",
    );
    if mobile {
        builder = builder.user_agent(MOBILE_UA);
    }
    let wv = window.add_child(builder, pos, size).map_err(err)?;
    #[cfg(target_os = "linux")]
    place_page(&wv, &rect);
    let _ = wv.set_zoom(zoom);
    state.0.lock().unwrap().browser_url = Some(u);
    Ok(())
}

/// A source reported a bot check that the hidden browser could not pass by itself (see
/// [`browser`]). The check is already waiting in the hidden window; this only remembers which source
/// to put back to work once it is passed, and repeats the notice for the UI.
#[tauri::command]
fn open_challenge(app: AppHandle, state: State<'_, AppState>, url: String, provider: String) -> CmdResult<()> {
    let host = url.split('/').nth(2).unwrap_or(&url).to_string();
    state.0.lock().unwrap().check_provider = Some(provider);
    if app.get_webview_window(browser::LABEL).is_some() {
        let _ = app.emit("check-needed", browser::CheckInfo { host });
    }
    Ok(())
}

/// Open the waiting bot check: the person chose to pass it.
#[tauri::command]
fn show_check(app: AppHandle) -> CmdResult<()> {
    browser::HiddenBrowser::show(&app).map_err(err)
}

/// Put the waiting bot check away; the source stays skipped for a while.
#[tauri::command]
fn dismiss_check(app: AppHandle) {
    browser::HiddenBrowser::hide(&app);
}

/// Switch the web panel between the phone and desktop layout. The page is rebuilt with the
/// other user agent; the UI re-opens it at the same address right after.
#[tauri::command]
fn browser_mobile(app: AppHandle, state: State<'_, AppState>, mobile: bool) {
    {
        let mut inner = state.0.lock().unwrap();
        inner.browser_mobile = mobile;
        inner.zoom = if mobile { 1.0 } else { 0.8 };
        inner.browser_url = None;
    }
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.close();
    }
}

#[tauri::command]
fn browser_hide(app: AppHandle) {
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.hide();
    }
}

#[tauri::command]
fn browser_back(app: AppHandle) {
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.eval("history.back()");
    }
}

#[tauri::command]
fn browser_forward(app: AppHandle) {
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.eval("history.forward()");
    }
}

#[tauri::command]
fn browser_reload(app: AppHandle) {
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.eval("location.reload()");
    }
}

/// Open the settings window, or bring it forward when it is already open.
#[tauri::command]
fn open_settings(app: AppHandle) -> CmdResult<()> {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        w.show().map_err(err)?;
        return w.set_focus().map_err(err);
    }
    tauri::WebviewWindowBuilder::new(&app, "settings", WebviewUrl::App("settings.html".into()))
        .title("Settings")
        .inner_size(880.0, 620.0)
        .min_inner_size(720.0, 480.0)
        .center()
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png")).map_err(err)?)
        .map_err(err)?
        .build()
        .map_err(err)?;
    Ok(())
}

/// Interface scale of the main window (Settings → Appearance), 0.8 to 1.5.
#[tauri::command]
fn set_ui_scale(app: AppHandle, scale: f64) -> CmdResult<()> {
    let ui = app.get_webview("ui").context("main window").map_err(err)?;
    ui.set_zoom(scale.clamp(0.8, 1.5)).map_err(err)
}

/// The text the person highlighted on the page in the web panel (empty when nothing is selected).
#[tauri::command]
async fn browser_selection(app: AppHandle) -> CmdResult<String> {
    let wv = app.get_webview("browser").context("no page is open").map_err(err)?;
    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = std::sync::Mutex::new(Some(tx));
    wv.eval_with_callback("window.__lxSel || String(window.getSelection ? window.getSelection() : '')", move |json| {
        if let Some(tx) = tx.lock().unwrap().take() {
            let _ = tx.send(json);
        }
    })
    .map_err(err)?;
    let json = tokio::time::timeout(std::time::Duration::from_secs(5), rx).await.map_err(|_| "the page did not answer".to_string())?.map_err(err)?;
    Ok(serde_json::from_str::<String>(&json).unwrap_or_default().trim().to_string())
}

#[tauri::command]
fn web_search_url(engine: String, artist: String, title: String) -> String {
    util::search_url(&engine, &artist, &title)
}

#[tauri::command]
fn open_external(url: String) -> CmdResult<()> {
    util::open_url(&url).map_err(err)
}

// ───────────────────────────── diagnostics (Settings → Diagnostics) ─────────────────────────────

#[derive(Serialize)]
struct Check {
    name: &'static str,
    /// "ok", "warn" or "bad"
    level: &'static str,
    detail: String,
}

fn on_path(tool: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(tool).is_file()))
}

/// Local checks for a bug report. Nothing here goes over the network ("Test all sources" does that),
/// and nothing secret is included: the proxy shows only its scheme and host.
#[tauri::command]
fn diagnose(app: AppHandle, state: State<'_, AppState>) -> Vec<Check> {
    use providers::plugin;
    let ok = |name, detail: String| Check { name, level: "ok", detail };
    let warn = |name, detail: String| Check { name, level: "warn", detail };
    let bad = |name, detail: String| Check { name, level: "bad", detail };
    let (cfg, write_error, engine_error) = {
        let inner = state.0.lock().unwrap();
        (inner.cfg.clone(), inner.settings_write_error.clone(), inner.engine.as_ref().err().cloned())
    };
    let mut out = vec![ok("Version", format!("Lyrixearch {} · {} {}", app.package_info().version, std::env::consts::OS, std::env::consts::ARCH))];

    let path = Config::path().map(|p| p.display().to_string()).unwrap_or_else(|| "unknown".into());
    out.push(match write_error {
        Some(e) => bad("Settings file", format!("{path}: {e}")),
        None => ok("Settings file", path),
    });

    let dir = Config::plugins_dir();
    let (loaded, failed) = plugin::discover(&dir);
    let ids: Vec<String> = loaded.iter().map(|p| p.manifest().id.clone()).collect();
    out.push(if !failed.is_empty() {
        let list = failed
            .iter()
            .map(|(p, e)| format!("{}: {e:#}", p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
            .collect::<Vec<_>>()
            .join("; ");
        bad("Plugins", format!("{} loaded, {} failed ({list})", loaded.len(), failed.len()))
    } else if loaded.is_empty() {
        ok("Plugins", "none installed".into())
    } else {
        let enabled = ids.iter().filter(|id| cfg.providers.contains(id)).count();
        ok("Plugins", format!("{} installed ({}), {enabled} enabled", loaded.len(), ids.join(", ")))
    });

    let known: Vec<String> = providers::builtin_ids().into_iter().chain(ids).collect();
    let unknown: Vec<&String> = cfg.providers.iter().filter(|p| !known.contains(p)).collect();
    out.push(match (&engine_error, unknown.is_empty(), cfg.providers.is_empty()) {
        (Some(e), _, _) => bad("Sources", e.clone()),
        (_, _, true) => bad("Sources", "none enabled: nothing will be found".into()),
        (_, false, _) => {
            warn("Sources", format!("{} enabled; not available: {}", cfg.providers.len(), unknown.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")))
        }
        _ => ok("Sources", format!("{} enabled: {}", cfg.providers.len(), cfg.providers.join(", "))),
    });

    out.push(match cfg.proxy.as_deref().filter(|p| !p.trim().is_empty()) {
        None => ok("Proxy", "none".into()),
        Some(p) => match reqwest::Url::parse(p) {
            Ok(u) => ok("Proxy", format!("{}://{}{}", u.scheme(), u.host_str().unwrap_or("?"), u.port().map(|p| format!(":{p}")).unwrap_or_default())),
            Err(_) => bad("Proxy", "not a valid address".into()),
        },
    });

    out.push(if app.try_state::<tray::Tray>().is_some() {
        ok("Menu bar icon", if cfg.keep_running { "shown; closing the window keeps the app running".into() } else { "shown".into() })
    } else {
        warn("Menu bar icon", "this desktop has no tray, so closing the window quits".into())
    });

    {
        use tauri_plugin_notification::{NotificationExt, PermissionState};
        out.push(match app.notification().permission_state() {
            _ if !cfg.notify_done => ok("Notifications", "turned off".into()),
            Ok(PermissionState::Granted) => ok("Notifications", "allowed".into()),
            Ok(PermissionState::Denied) => warn("Notifications", "blocked in the system settings".into()),
            Ok(_) => ok("Notifications", "the system asks the first time".into()),
            Err(e) => warn("Notifications", e.to_string()),
        });
    }

    let awake_tool = if cfg!(target_os = "macos") {
        Some(std::path::Path::new("/usr/bin/caffeinate").is_file())
    } else if cfg!(target_os = "linux") {
        Some(on_path("systemd-inhibit"))
    } else {
        None
    };
    out.push(match (cfg.keep_awake, awake_tool) {
        (false, _) => ok("Keep awake", "turned off".into()),
        (true, Some(false)) => warn("Keep awake", "systemd-inhibit not found: the computer may sleep during a fetch".into()),
        _ => ok("Keep awake", "on while fetching".into()),
    });

    out.push(ok(
        "Bot checks",
        if cfg!(target_os = "linux") {
            "WebKitGTK, in a hidden window".into()
        } else if cfg!(windows) {
            "WebView2, in a hidden window".into()
        } else {
            "WebKit, in a hidden window".into()
        },
    ));
    out
}

// ───────────────────────────── AI transcription (Settings → Transcription) ─────────────────────────────

/// What local transcription has to work with: whisper.cpp, ffmpeg and a model.
#[tauri::command]
fn transcribe_status(state: State<'_, AppState>) -> serde_json::Value {
    use providers::transcribe;
    let cfg = state.0.lock().unwrap().cfg.clone();
    let path = |p: Option<PathBuf>| p.map(|p| p.display().to_string());
    serde_json::json!({
        "whisper": path(transcribe::whisper_bin(&cfg)),
        "ffmpeg": path(transcribe::find_tool("ffmpeg")),
        "model": path(transcribe::model_path(&cfg)),
        "model_name": transcribe::MODEL_NAME,
        "models_dir": transcribe::models_dir().display().to_string(),
    })
}

/// Download the recommended whisper.cpp model into the models folder, reporting progress as
/// `model-download` events (`{ done, total }`). A half-finished download never counts as a model.
#[tauri::command]
async fn transcribe_download_model(app: AppHandle) -> CmdResult<String> {
    use providers::transcribe;
    use std::io::Write;
    let dir = transcribe::models_dir();
    std::fs::create_dir_all(&dir).map_err(err)?;
    let dest = dir.join(transcribe::MODEL_NAME);
    let part = dir.join(format!("{}.part", transcribe::MODEL_NAME));
    let mut resp = reqwest::Client::new().get(transcribe::MODEL_URL).send().await.map_err(err)?.error_for_status().map_err(err)?;
    let total = resp.content_length().unwrap_or(0);
    let mut file = std::fs::File::create(&part).map_err(err)?;
    let mut done: u64 = 0;
    let mut last = std::time::Instant::now();
    while let Some(chunk) = resp.chunk().await.map_err(err)? {
        file.write_all(&chunk).map_err(err)?;
        done += chunk.len() as u64;
        if last.elapsed() > std::time::Duration::from_millis(250) {
            let _ = app.emit("model-download", serde_json::json!({ "done": done, "total": total }));
            last = std::time::Instant::now();
        }
    }
    file.flush().map_err(err)?;
    drop(file);
    if total > 0 && done != total {
        let _ = std::fs::remove_file(&part);
        return Err(format!("the download stopped early ({done} of {total} bytes)"));
    }
    std::fs::rename(&part, &dest).map_err(err)?;
    let _ = app.emit("model-download", serde_json::json!({ "done": done, "total": total, "finished": true }));
    Ok(dest.display().to_string())
}

// ───────────────────────────── accounts (Settings → Accounts) ─────────────────────────────

#[tauri::command]
fn account_list() -> Vec<serde_json::Value> {
    accounts::list()
}

/// `{ signedIn, name }` for one service; signing in happens on the service's own page.
#[tauri::command]
async fn account_status(accounts: State<'_, std::sync::Arc<accounts::Accounts>>, id: String) -> CmdResult<serde_json::Value> {
    match accounts.run(&id, "status", serde_json::json!({}), std::time::Duration::from_secs(20)).await {
        Ok(v) => Ok(v),
        Err(e) => Ok(serde_json::json!({ "signedIn": false, "error": format!("{e:#}") })),
    }
}

#[tauri::command]
fn account_sign_in(app: AppHandle, id: String) -> CmdResult<()> {
    accounts::Accounts::sign_in(&app, &id).map_err(err)
}

#[tauri::command]
async fn account_sign_out(app: AppHandle, accounts: State<'_, std::sync::Arc<accounts::Accounts>>, id: String) -> CmdResult<()> {
    accounts.run(&id, "signout", serde_json::json!({}), std::time::Duration::from_secs(15)).await.map_err(err)?;
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    let _ = app.emit("accounts-changed", ());
    Ok(())
}

/// Show the settings file in Finder / the file manager.
#[tauri::command]
fn reveal_config() -> CmdResult<()> {
    let path = Config::path().context("no settings location").map_err(err)?;
    util::reveal(&path).map_err(err)
}

// ───────────────────────────── cover art, file manager, sidecar files ─────────────────────────────

fn folder_cover(dir: &std::path::Path) -> Option<Vec<u8>> {
    let names = ["cover", "folder", "front", "album", "albumart", "artwork"];
    let exts = ["jpg", "jpeg", "png", "webp"];
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        let stem = p.file_stem()?.to_string_lossy().to_lowercase();
        let ext = p.extension()?.to_string_lossy().to_lowercase();
        if names.contains(&stem.as_str()) && exts.contains(&ext.as_str()) {
            return std::fs::read(&p).ok();
        }
    }
    None
}

fn thumbnail(path: &std::path::Path) -> Option<String> {
    use base64::Engine as _;
    let bytes = tags::read_cover(path).or_else(|| folder_cover(path.parent()?))?;
    let img = image::load_from_memory(&bytes).ok()?.thumbnail(128, 128).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80).encode_image(&img).ok()?;
    Some(format!("data:image/jpeg;base64,{}", base64::engine::general_purpose::STANDARD.encode(out)))
}

/// Cover thumbnail as a data URL: the embedded picture, else cover.jpg / folder.jpg in the folder.
#[tauri::command]
async fn get_cover(state: State<'_, AppState>, id: u64) -> CmdResult<Option<String>> {
    let (path, key) = {
        let inner = state.0.lock().unwrap();
        let e = inner.entries.iter().find(|e| e.id == id).context("unknown track").map_err(err)?;
        let key = format!(
            "{}|{}",
            e.track.path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default(),
            e.track.query.album.clone().unwrap_or_default()
        );
        if let Some(hit) = inner.covers.get(&key) {
            return Ok(hit.clone());
        }
        (e.track.path.clone(), key)
    };
    let thumb = tauri::async_runtime::spawn_blocking(move || thumbnail(&path)).await.map_err(err)?;
    state.0.lock().unwrap().covers.insert(key, thumb.clone());
    Ok(thumb)
}

#[tauri::command]
fn reveal_track(state: State<'_, AppState>, id: u64) -> CmdResult<()> {
    let path = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(|e| e.track.path.clone()).context("unknown track").map_err(err)?;
    util::reveal(&path).map_err(err)
}

#[derive(Serialize)]
struct SidecarResult {
    embedded: usize,
    /// No matching .lrc / .txt file next to the track.
    missing: usize,
    /// Left alone because the track already has lyrics and replacing is off.
    kept: usize,
    failed: usize,
}

/// Embed the `.lrc` / `.txt` that sits next to each track (same name, same folder or a Lyrics folder).
/// `kind` limits it to one of the two; `replace` allows overwriting lyrics that are already embedded.
#[tauri::command]
async fn embed_sidecar(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>, kind: Option<String>, replace: Option<bool>) -> CmdResult<SidecarResult> {
    let replace = replace.unwrap_or(true);
    let (jobs, cfg) = {
        let inner = state.0.lock().unwrap();
        let ids: HashSet<u64> = ids.into_iter().collect();
        (inner.entries.iter().filter(|e| ids.contains(&e.id)).map(|e| (e.id, e.track.clone())).collect::<Vec<_>>(), inner.cfg.clone())
    };
    let mut res = SidecarResult { embedded: 0, missing: 0, kept: 0, failed: 0 };
    for (id, track) in jobs {
        let cfg = cfg.clone();
        let kind = kind.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<_> {
            let outcome = lyrixearch_core::output::embed_from_sidecar(&track, &cfg, replace, kind.as_deref(), false)?;
            let embedded = if matches!(outcome, SidecarEmbed::Embedded(_)) { tags::read_lyrics(&track.path) } else { None };
            Ok((outcome, embedded))
        })
        .await
        .map_err(err)?;
        match outcome {
            Ok((SidecarEmbed::Embedded(_), embedded)) => {
                res.embedded += 1;
                set(&state, id, |e| {
                    e.track.has_lyrics = true;
                    e.embedded = embedded;
                    if e.found.take().is_some() {
                        forget_pending(&app, &[util::pending_key(&e.track.path)]);
                    }
                    e.provider = Some("file".into());
                    e.status = Status::Written;
                    e.note = None;
                });
            }
            Ok((SidecarEmbed::NoFile, _)) => res.missing += 1,
            Ok((SidecarEmbed::AlreadyHad, _)) => res.kept += 1,
            Ok((SidecarEmbed::WouldEmbed(_), _)) => unreachable!("dry run is disabled"),
            Err(m) => {
                res.failed += 1;
                set(&state, id, |e| {
                    e.status = Status::Error;
                    e.note = Some(format!("{m:#}"));
                });
            }
        }
        emit_track(&app, &state, id);
    }
    let _ = app.emit("stats", stats(&state.0.lock().unwrap()));
    Ok(res)
}

#[derive(Serialize)]
struct ExportResult {
    written: usize,
    /// A file with that name already exists and replacing is off.
    kept: usize,
    /// No lyrics for the track, or no synced lyrics when `.lrc` was asked for.
    none: usize,
    failures: Vec<String>,
}

/// Write each track's lyrics (found or embedded) to a `.lrc` / `.txt` next to it.
/// `kind` is `auto` (`.lrc` when synced, else `.txt`), `lrc` or `txt`.
#[tauri::command]
async fn export_lyrics(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>, kind: Option<String>, replace: Option<bool>) -> CmdResult<ExportResult> {
    let mode = match kind.as_deref() {
        Some("lrc") => lyrixearch_core::Sidecar::Lrc,
        Some("txt") => lyrixearch_core::Sidecar::Txt,
        _ => lyrixearch_core::Sidecar::Auto,
    };
    let replace = replace.unwrap_or(true);
    let ids: HashSet<u64> = ids.into_iter().collect();
    let (jobs, cfg) = {
        let inner = state.0.lock().unwrap();
        let jobs = inner
            .entries
            .iter()
            .filter(|e| ids.contains(&e.id))
            .map(|e| {
                let lyrics = e.found.clone().or_else(|| e.embedded.as_ref().map(|text| Lyrics { plain: Some(text.clone()), ..Default::default() }));
                (e.track.path.clone(), lyrics, e.found.is_some())
            })
            .collect::<Vec<_>>();
        (jobs, inner.cfg.clone())
    };
    let (res, exported) = tauri::async_runtime::spawn_blocking(move || {
        use std::io::{ErrorKind, Write};
        let mut res = ExportResult { written: 0, kept: 0, none: 0, failures: Vec::new() };
        let mut exported = Vec::new();
        for (audio, mut lyrics, found) in jobs {
            // embedded lyrics: pick up the timed copy too, so Auto / .lrc can use it
            if let Some(l) = lyrics.as_mut().filter(|l| !found && l.synced.is_none()) {
                l.synced = tags::read_synced(&audio);
            }
            let Some((path, body)) = lyrics.and_then(|lyrics| lyrixearch_core::output::sidecar_plan(&audio, &lyrics, mode, &cfg)) else {
                res.none += 1;
                continue;
            };
            let written = if replace {
                lyrixearch_core::atomic::write(&path, body.as_bytes())
            } else {
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                    Ok(mut file) => file.write_all(body.as_bytes()).map_err(anyhow::Error::from),
                    Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                        res.kept += 1;
                        continue;
                    }
                    Err(e) => Err(e.into()),
                }
            };
            match written {
                Ok(()) => {
                    res.written += 1;
                    if found {
                        exported.push(util::pending_key(&audio));
                    }
                }
                Err(e) => res.failures.push(format!("{}: {e:#}", path.display())),
            }
        }
        (res, exported)
    })
    .await
    .map_err(err)?;
    forget_pending(&app, &exported);
    Ok(res)
}

// ───────────────────────────── playback ─────────────────────────────

#[tauri::command]
fn player_load(player: State<'_, player::Player>, state: State<'_, AppState>, id: u64, play: bool) -> CmdResult<()> {
    let (path, hint) = {
        let inner = state.0.lock().unwrap();
        let e = inner.entries.iter().find(|e| e.id == id).context("unknown track").map_err(err)?;
        (e.track.path.clone(), e.track.query.duration_secs.map(f64::from))
    };
    player.send(player::Cmd::Load { id, path, play, hint });
    Ok(())
}

#[tauri::command]
fn player_toggle(player: State<'_, player::Player>) {
    player.send(player::Cmd::Toggle);
}

#[tauri::command]
fn player_seek(player: State<'_, player::Player>, secs: f64) {
    player.send(player::Cmd::Seek(secs));
}

#[tauri::command]
fn player_stop(player: State<'_, player::Player>) {
    player.send(player::Cmd::Stop);
}

// ───────────────────────────── files handed to the app by the OS ─────────────────────────────

fn opened_from_args() -> Vec<String> {
    std::env::args().skip(1).filter(|a| !a.starts_with('-') && std::path::Path::new(a).exists()).collect()
}

/// Paths from a drop on the app icon, "Open with", or the command line; the UI collects them.
#[tauri::command]
fn take_pending_open(state: State<'_, AppState>) -> Vec<String> {
    std::mem::take(&mut state.0.lock().unwrap().pending_open)
}

#[tauri::command]
fn browser_zoom(app: AppHandle, state: State<'_, AppState>, delta: f64) {
    let mut inner = state.0.lock().unwrap();
    inner.zoom = (inner.zoom + delta).clamp(0.4, 2.0);
    if let Some(wv) = app.get_webview("browser") {
        let _ = wv.set_zoom(inner.zoom);
    }
}

/// Self-test progress, so a step that hangs shows in the log.
#[tauri::command]
fn selftest_step(step: String) {
    if std::env::var_os("LYRIXEARCH_SELFTEST").is_some() {
        eprintln!("SELFTEST step: {step}");
    }
}

/// Used only by the built-in self-test (`LYRIXEARCH_SELFTEST=1`): prints what the UI observed
/// together with the real state of the embedded browser webview, then quits.
#[tauri::command]
fn selftest_report(app: AppHandle, info: String) {
    if std::env::var_os("LYRIXEARCH_SELFTEST").is_none() {
        return;
    }
    eprintln!("SELFTEST ui: {info}");
    match app.get_webview("browser") {
        Some(wv) => {
            eprintln!("SELFTEST browser webview: position={:?} size={:?} url={:?}", wv.position().ok(), wv.size().ok(), wv.url().ok().map(|u| u.to_string()))
        }
        None => eprintln!("SELFTEST browser webview: not created"),
    }
    {
        use reqwest::cookie::CookieStore;
        let u: tauri::Url = "https://httpbin.org/".parse().unwrap();
        eprintln!(
            "SELFTEST challenge cookies in the engine jar: {:?}, challenge window open: {}",
            jar().cookies(&u),
            app.get_webview_window(browser::LABEL).is_some()
        );
    }
    if let Some(w) = app.get_window("main") {
        eprintln!("SELFTEST window: inner={:?} scale={:?} webviews={}", w.inner_size().ok(), w.scale_factor().ok(), w.webviews().len());
    }
    // Shrink the window, then check the browser panel still sits where the UI put it (macOS pins child
    // webviews to the bottom edge, so without the resize handler it would slide up).
    let h = app.clone();
    std::thread::spawn(move || {
        let want = h.state::<AppState>().0.lock().unwrap().browser_rect;
        if let (Some(w), Some(r)) = (h.get_window("main"), want) {
            let _ = w.set_size(LogicalSize::new(1180.0, 640.0));
            std::thread::sleep(std::time::Duration::from_millis(1200));
            if let Some(wv) = h.get_webview("browser") {
                let p = wv.position().ok().and_then(|p| p.to_logical::<f64>(w.scale_factor().unwrap_or(1.0)).into());
                let p: Option<tauri::LogicalPosition<f64>> = p;
                eprintln!("SELFTEST after-resize: browser at {:?}, wanted ({}, {}), inner={:?}", p.map(|p| (p.x, p.y)), r.x, r.y, w.inner_size().ok());
            }
        }
        h.exit(0);
    });
}

#[cfg_attr(not(debug_assertions), allow(dead_code))]
const SELFTEST_JS: &str = r#"
(async () => {
  const out = { jsErrors: [] };
  window.addEventListener('error', (e) => out.jsErrors.push(String(e.message)));
  window.addEventListener('unhandledrejection', (e) => out.jsErrors.push(String(e.reason && (e.reason.message || e.reason))));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a || {});
  const step = (s) => inv('selftest_step', { step: s }).catch(() => {});
  step('start');
  try {
    out.tauri = !!window.__TAURI__;
    out.mock = !!window.__MOCK__;
    await Promise.race([document.fonts.ready, sleep(3000)]);
    step('fonts');
    out.font = getComputedStyle(document.body).fontFamily.split(',')[0];
    out.layout = { inner: innerWidth, detail: Math.round(document.getElementById('detail').getBoundingClientRect().width), cssVar: getComputedStyle(document.getElementById('app')).getPropertyValue('--detail-w').trim(), saved: (() => { try { return localStorage.getItem('lf-detail-w'); } catch { return 'n/a'; } })() };
    out.noWordmark = !document.querySelector('.wordmark');
    await sleep(1500);                         // the drop event was delivered by the backend
    const S = window.__lf.state;
    const tracks = [...S.tracks.values()];
    out.tracks = tracks.length;
    out.rows = document.querySelectorAll('.row').length;
    const by = (re) => tracks.find((t) => re.test(t.name));

    // search through the real command, with the camelCase argument the UI sends
    const plain = by(/plain/i) || tracks[0];
    step(`tracks ${tracks.length}, searching ${plain && plain.name}`);
    await inv('search', { ids: [plain.id], autoWrite: false });
    for (let i = 0; i < 80; i++) { await sleep(500); const p = S.progress; if (p && !p.running && p.total > 0) break; }
    out.search = { status: S.tracks.get(plain.id).status, via: S.tracks.get(plain.id).provider, done: S.progress && S.progress.done };

    step('searched');
    // cover thumbnail (embedded art)
    const cov = by(/cover/i);
    if (cov) { const c = await inv('get_cover', { id: cov.id }); out.cover = c ? c.slice(0, 23) + ' len=' + c.length : null; }

    // embed from a matching .lrc next to the track
    const sc = by(/sidecar/i);
    if (sc) { out.sidecar = await inv('embed_sidecar', { ids: [sc.id] }); out.sidecarState = S.tracks.get(sc.id).has_lyrics; }

    step('player');
    // player: decode, seek, play briefly, stop
    const pl = by(/player/i) || tracks[0];
    const events = [];
    window.__TAURI__.event.listen('player', (e) => events.push(e.payload));
    await inv('player_load', { id: pl.id, play: false });
    await sleep(500);
    const loaded = events[events.length - 1];
    await inv('player_seek', { secs: 1 });
    await sleep(300);
    await inv('player_toggle');
    await sleep(900);
    const playing = events[events.length - 1];
    await inv('player_stop');
    await sleep(300);
    out.player = { loaded: loaded && { state: loaded.state, duration: loaded.duration, error: loaded.error }, playing: playing && { state: playing.state, pos: Math.round(playing.pos * 10) / 10, error: playing.error }, stopped: events[events.length - 1].state };

    step('details');
    // layouts and the library pane, each held for a moment so a watcher can take a screenshot
    const before = localStorage.getItem('lf-layout');
    const pick = async (sel) => { document.getElementById('btn-view').click(); await sleep(200); document.querySelector(sel).click(); await sleep(300); };
    window.__lf.selectId(null);
    document.querySelector('#pane-tabs [data-pane=overview]').click();
    await sleep(400);
    out.overview = [...document.querySelectorAll('#overview .dx-card h3')].map((h) => h.textContent.trim());
    step('shot:comfortable'); await sleep(2500);
    await pick('[data-layout=compact]'); step('shot:compact'); await sleep(2500);
    await pick('[data-layout=albums]');
    out.albumHeaders = document.querySelectorAll('#rows .grp').length;
    step('shot:albums'); await sleep(2500);
    document.querySelector('#pane-tabs [data-pane=activity]').click(); await sleep(400);
    out.activity = [...document.querySelectorAll('#overview .dx-card h3')].map((h) => h.textContent.trim());
    step('shot:activity'); await sleep(2500);
    await pick(`[data-layout=${before || 'comfortable'}]`);
    // the inspector: Details (search trail, match, lyrics), then Tidy, then the embedded browser
    window.__lf.selectId(plain.id);
    step('selected');
    await sleep(500);
    step('slept');
    document.querySelector('[data-tab=info]').click();
    step('info clicked');
    await sleep(600);
    step('info shown');
    const info = document.getElementById('info');
    out.details = {
      shown: !document.getElementById('tab-info').hidden,
      cards: [...info.querySelectorAll('.dx-card h3')].map((h) => h.textContent.trim()),
      trail: [...info.querySelectorAll('.dx-trail li')].map((li) => li.textContent.replace(/\s+/g, ' ').trim()),
      gauge: !!info.querySelector('.gauge'),
      stats: [...info.querySelectorAll('.dx-stat')].map((x) => x.textContent.replace(/\s+/g, ' ').trim()),
    };
    document.querySelector('[data-tab=lyrics]').click();
    await sleep(300);
    step('tools');
    document.getElementById('act-tools').click();
    await sleep(300);
    step('tools open');
    out.tidyMenu = !document.getElementById('tools-pop').hidden;
    document.querySelector('[data-tool="spaces"]').click();
    step('tidy clicked');
    await sleep(400);
    step('tidy done');
    out.tidyOpensEdit = !document.getElementById('tab-edit').hidden;
    document.querySelector('[data-tab=web]').click();
    step('web clicked');
    await sleep(4000);
    step('web shown');
    const r = document.getElementById('webslot').getBoundingClientRect();
    out.slot = { x: r.left, y: r.top, w: r.width, h: r.height };

    // AI search modes: the exact URLs the UI builds (Google AI Mode opens in the real browser, so only the URL is checked)
    out.aiUrl = window.__lf.urlFor('Queen - Bohemian Rhapsody lyrics', 'google_ai');
    out.braveUrl = window.__lf.urlFor('Queen - Bohemian Rhapsody lyrics', 'brave_ai');

    // mobile view: point the panel at a local server that records the User-Agent it receives
    const uaUrl = '__UA_URL__';
    if (uaUrl) {
      document.getElementById('web-query').value = uaUrl;
      document.getElementById('web-go').click();
      await sleep(2500);
      out.mobilePressed = document.getElementById('web-mobile').getAttribute('aria-pressed');
      document.getElementById('web-mobile').click();      // switch to the desktop layout
      await sleep(3000);
      out.desktopPressed = document.getElementById('web-mobile').getAttribute('aria-pressed');
    }
    step('web done');
  } catch (e) { out.error = String((e && e.message) || e); }
  step('challenge');
  try {
    // verification window: a page that sets cf_clearance should be picked up and the window closed by itself
    await inv('open_challenge', { url: 'https://httpbin.org/cookies/set?cf_clearance=selftest', provider: 'selftest' });
    await sleep(6000);
  } catch (e) { out.challengeErr = String(e); }
  try { await window.__TAURI__.core.invoke('selftest_report', { info: JSON.stringify(out) }); } catch (e) { document.title = 'selftest failed: ' + e; }
})();
"#;

fn config_load_detail(error: &anyhow::Error) -> String {
    let cause = error.root_cause().to_string();
    if let Some(location) = cause.lines().next().filter(|line| line.starts_with("TOML parse error at line ")) {
        // The following lines from TOML can show the original value, including API keys.
        return location.to_string();
    }
    if let Some(io_error) = error.downcast_ref::<std::io::Error>() {
        return io_error.to_string();
    }
    let detail = error.chain().find_map(|cause| {
        let text = cause.to_string();
        ["tag_clean_pattern is not a valid regex", "min_score must be between 0 and 1", "invalid proxy url"]
            .into_iter()
            .find(|message| text.starts_with(message))
    });
    detail.unwrap_or("Invalid settings file").into()
}

fn main() {
    // WebKitGTK's DMA-BUF renderer fails on many Wayland/Mesa/NVIDIA setups with "Error 71 (Protocol error)";
    // switching it off is harmless elsewhere. A user's own setting wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    let mut settings_write_error = None;
    let mut notices = Vec::new();
    let cfg = Config::load().unwrap_or_else(|error| {
        let notice = format!("Could not load settings ({}).", config_load_detail(&error));
        match Config::quarantine_unreadable() {
            Ok(Some(path)) => notices.push(format!("{notice} The original was moved to {}. Using defaults.", path.display())),
            Ok(None) => notices.push(format!("{notice} Using defaults.")),
            Err(error) => {
                let message = format!("{notice} Could not back up the file. Settings saves are disabled for this session. {error:#}");
                notices.push(message.clone());
                settings_write_error = Some(message);
            }
        }
        Config::default()
    });
    let engine = Engine::with_cookies(cfg.clone(), jar()).map_err(|error| format!("Could not start the search engine. Review your settings. {error:#}"));
    if let Err(error) = &engine {
        notices.push(error.clone());
    }
    let state = AppState(Mutex::new(Inner {
        cfg,
        engine,
        settings_write_error,
        startup_notice: (!notices.is_empty()).then(|| notices.join("\n")),
        entries: Vec::new(),
        next_id: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        browser_url: None,
        covers: HashMap::new(),
        pending_open: opened_from_args(),
        zoom: 1.0,
        browser_mobile: true,
        browser_rect: None,
        check_provider: None,
    }));

    let mut builder = tauri::Builder::default();
    // Linux and Windows: a second launch (a file dropped on the launcher, "Open with") hands its
    // files to the running app instead of opening another window. macOS does this itself. The
    // built-in tests run their own copy next to one that is open, so they leave this out.
    let testing = std::env::var_os("LYRIXEARCH_SELFTEST").is_some() || std::env::var_os("LYRIXEARCH_CHECKTEST").is_some();
    if !testing {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            let paths: Vec<String> = argv.iter().skip(1).filter(|a| !a.starts_with('-') && std::path::Path::new(a).exists()).cloned().collect();
            if !paths.is_empty() {
                app.state::<AppState>().0.lock().unwrap().pending_open.extend(paths);
                let _ = app.emit("open-paths", ());
            }
            background::show_main(app);
        }));
    }
    builder
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            list_tracks,
            add_paths,
            remove_tracks,
            clear_tracks,
            search,
            cancel_search,
            get_lyrics,
            apply_found,
            save_lyrics,
            delete_lyrics,
            get_settings,
            set_settings,
            install_plugin,
            test_providers,
            browser_show,
            browser_hide,
            browser_back,
            browser_forward,
            browser_reload,
            browser_selection,
            open_settings,
            diagnose,
            reveal_config,
            account_list,
            transcribe_status,
            transcribe_download_model,
            account_status,
            account_sign_in,
            account_sign_out,
            set_ui_scale,
            web_search_url,
            open_external,
            selftest_report,
            selftest_step,
            get_cover,
            reveal_track,
            embed_sidecar,
            export_lyrics,
            player_load,
            player_toggle,
            player_seek,
            player_stop,
            take_pending_open,
            browser_zoom,
            browser_mobile,
            open_challenge,
            show_check,
            dismiss_check,
        ])
        .setup(|app| {
            app.manage(player::Player::start(app.handle().clone()));
            // sources behind bot checks load their pages through a hidden browser window
            lyrixearch_core::providers::web::set_browser(std::sync::Arc::new(browser::HiddenBrowser::new(app.handle().clone())));
            // signed-in sessions for Spotify, Apple Music and Deezer (Settings → Accounts)
            let accounts = std::sync::Arc::new(accounts::Accounts::new(app.handle().clone()));
            lyrixearch_core::providers::accounts::set_bridge(accounts.clone());
            accounts::Accounts::start_idle_close(accounts.clone());
            app.manage(accounts);
            {
                use tauri::Listener;
                let h = app.handle().clone();
                app.handle().listen("check-cleared", move |_| {
                    let state = h.state::<AppState>();
                    let mut inner = state.0.lock().unwrap();
                    let provider = inner.check_provider.take();
                    if let Ok(engine) = &inner.engine {
                        if let Some(p) = provider {
                            engine.clear_penalty(&p);
                        }
                        engine.forget_announced();
                    }
                });
            }
            if let Err(e) = tray::install(app.handle()) {
                // some Linux desktops have no tray; the app works the same without it
                eprintln!("lyrixearch: no tray icon: {e}");
            } else {
                #[cfg(target_os = "macos")]
                if let Err(e) = shelf::install(app.handle()) {
                    eprintln!("lyrixearch: no menu bar drop zone: {e}");
                }
            }
            let builder = WindowBuilder::new(app, "main")
                .title("Lyrixearch")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))?)?
                .inner_size(1380.0, 880.0)
                .min_inner_size(980.0, 620.0);
            // macOS: one unified toolbar, as native apps have: the window buttons sit in the app's own
            // header and there is no separate title strip repeating the name
            #[cfg(target_os = "macos")]
            let builder = builder.title_bar_style(tauri::TitleBarStyle::Overlay).hidden_title(true);
            let window = builder.build()?;
            #[cfg(target_os = "macos")]
            background::place_window_buttons(&window);
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            window.add_child(WebviewBuilder::new("ui", WebviewUrl::App("index.html".into())), LogicalPosition::new(0.0, 0.0), size)?;

            // Debug builds only: search one song through the configured sources and print the outcome
            // (LYRIXEARCH_CHECKTEST=1, used to check sources behind bot checks with the hidden browser).
            #[cfg(debug_assertions)]
            if std::env::var_os("LYRIXEARCH_CHECKTEST").is_some() {
                let h = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let engine = h.state::<AppState>().0.lock().unwrap().engine.clone();
                    let Ok(engine) = engine else { return };
                    // LYRIXEARCH_CHECKTEST="Artist|Title|Album" picks the song; anything else searches Coldplay - Yellow
                    let spec = std::env::var("LYRIXEARCH_CHECKTEST").unwrap_or_default();
                    let parts: Vec<&str> = spec.split('|').collect();
                    let q = if parts.len() >= 2 {
                        lyrixearch_core::Query {
                            artist: parts[0].into(),
                            title: parts[1].into(),
                            album: parts.get(2).map(|a| a.to_string()),
                            ..Default::default()
                        }
                    } else {
                        lyrixearch_core::Query { artist: "Coldplay".into(), title: "Yellow".into(), duration_secs: Some(269), ..Default::default() }
                    };
                    let t = std::time::Instant::now();
                    let out = engine.find(&q, |_, _| {}).await;
                    let summary = match &out {
                        lyrixearch_core::Outcome::Found(l) => format!("found via {} ({} chars)", l.source, l.plain.as_deref().map_or(0, str::len)),
                        other => format!("{other:?}"),
                    };
                    eprintln!("CHECKTEST {summary} in {:.1}s", t.elapsed().as_secs_f32());
                    // which windows exist and which a person can see (the hidden ones must stay hidden)
                    for (label, w) in h.webview_windows() {
                        eprintln!("CHECKTEST window {label}: visible={:?}", w.is_visible().ok());
                    }
                    h.exit(0);
                });
            }

            // Debug builds only: the built-in native self-test (LYRIXEARCH_SELFTEST=1).
            #[cfg(debug_assertions)]
            {
                if std::env::var_os("LYRIXEARCH_SELFTEST").is_some() {
                    let h = app.handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_secs(4));
                        match h.get_webview("ui") {
                            Some(ui) => {
                                // Deliver a drop exactly as the OS layer would: through the UI's drag-drop listener.
                                let dir = std::env::var("LYRIXEARCH_SELFTEST_DIR").unwrap_or_else(|_| "/tmp/lf-test".into());
                                let _ = h.emit_to("ui", "tauri://drag-drop", serde_json::json!({ "paths": [dir], "position": { "x": 0, "y": 0 } }));
                                if let Err(e) =
                                    ui.eval(SELFTEST_JS.replace("__UA_URL__", std::env::var("LYRIXEARCH_SELFTEST_UA_URL").unwrap_or_default().as_str()))
                                {
                                    eprintln!("SELFTEST could not run script: {e}");
                                    h.exit(1);
                                }
                            }
                            None => {
                                eprintln!("SELFTEST ui webview missing");
                                h.exit(1);
                            }
                        }
                        // Safety net: never hang if the UI cannot report back.
                        std::thread::sleep(std::time::Duration::from_secs(180));
                        eprintln!("SELFTEST timed out waiting for the UI");
                        h.exit(2);
                    });
                }
            }

            // The UI webview always fills the window; the browser panel is positioned by the UI.
            let handle = app.handle().clone();
            window.on_window_event(move |ev| {
                // with "keep running" on, closing the window leaves the app in the menu bar / tray
                // (only when the tray icon exists, or the window could not be brought back)
                if let WindowEvent::CloseRequested { api, .. } = ev {
                    let keep = handle.state::<AppState>().0.lock().unwrap().cfg.keep_running;
                    if keep && handle.try_state::<tray::Tray>().is_some() {
                        api.prevent_close();
                        background::hide_main(&handle);
                        return;
                    }
                }
                // the hidden check window must not keep the app alive once the main window is gone
                if let WindowEvent::Destroyed = ev {
                    handle.exit(0);
                }
                if let WindowEvent::ThemeChanged(_) = ev {
                    tray::refresh_theme(&handle);
                }
                #[cfg(target_os = "macos")]
                if matches!(ev, WindowEvent::Resized(_) | WindowEvent::ThemeChanged(_) | WindowEvent::Focused(_)) {
                    // AppKit puts the window buttons back in its own title strip on these
                    if let Some(w) = handle.get_window("main") {
                        background::place_window_buttons(&w);
                    }
                }
                if let WindowEvent::Resized(_) = ev {
                    if let (Some(w), Some(ui)) = (handle.get_window("main"), handle.get_webview("ui")) {
                        if let (Ok(s), Ok(f)) = (w.inner_size(), w.scale_factor()) {
                            let _ = ui.set_size(s.to_logical::<f64>(f));
                        }
                        // macOS pins child webviews to the window's bottom edge, so a height change slides the
                        // browser panel away from its slot. Put it back where the UI last placed it.
                        if let (Some(wv), Some(r)) = (handle.get_webview("browser"), handle.state::<AppState>().0.lock().unwrap().browser_rect) {
                            let _ = wv.set_position(page_position(&w, &r));
                            let _ = wv.set_size(LogicalSize::new((r.w * r.zoom()).max(1.0), (r.h * r.zoom()).max(1.0)));
                        }
                    }
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Lyrixearch")
        .run(|app, event| {
            // macOS: a folder or file dropped on the Dock/Finder icon, or "Open with", arrives here.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Opened { urls } = &event {
                let paths: Vec<String> = urls.iter().filter_map(|u| u.to_file_path().ok()).map(|p| p.to_string_lossy().into_owned()).collect();
                if !paths.is_empty() {
                    app.state::<AppState>().0.lock().unwrap().pending_open.extend(paths);
                    let _ = app.emit("open-paths", ());
                    background::show_main(app);
                }
            }
            // macOS: clicking the Dock icon while the window is hidden brings it back
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { has_visible_windows: false, .. } = &event {
                background::show_main(app);
            }
            #[cfg(not(target_os = "macos"))]
            let _ = (app, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toml_notice_preserves_location_without_echoing_config_values() -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!("lyrixearch-gui-config-{}", std::process::id()));
        std::fs::create_dir(&dir)?;
        let path = dir.join("config.toml");
        std::fs::write(&path, "lastfm_api_key = \"synthetic-fixture\" invalid\n")?;
        let error = Config::load_from(&path).unwrap_err();
        std::fs::remove_dir_all(dir)?;
        let detail = config_load_detail(&error);
        assert!(detail.starts_with("TOML parse error at line 1, column "));
        assert!(!detail.contains("synthetic-fixture"));
        Ok(())
    }
}
