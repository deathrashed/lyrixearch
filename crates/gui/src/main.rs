// Prevents an extra console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod player;

use anyhow::Context as _;
use lyrixearch_core::providers::{self, ProviderInfo};
use lyrixearch_core::{scan, tags, util, Config, Engine, Event, Lyrics, Track};
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
}

struct Inner {
    cfg: Config,
    engine: Arc<Engine>,
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
    let with = inner.entries.iter().filter(|e| e.track.has_lyrics).count();
    TrackList {
        tracks: inner.entries.iter().map(dto).collect(),
        stats: Stats { total: inner.entries.len(), with_lyrics: with, missing: inner.entries.len() - with },
        skipped,
    }
}

// ───────────────────────────── library ─────────────────────────────

#[tauri::command]
fn list_tracks(state: State<'_, AppState>) -> TrackList {
    list(&state.0.lock().unwrap(), 0)
}

/// Scan files/folders (recursively) and add the audio files to the working list.
#[tauri::command]
async fn add_paths(state: State<'_, AppState>, paths: Vec<String>) -> CmdResult<TrackList> {
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
    for (track, embedded) in loaded {
        let id = inner.next_id;
        inner.next_id += 1;
        inner.entries.push(Entry { id, track, status: Status::Idle, provider: None, found: None, embedded, note: None });
    }
    Ok(list(&inner, skipped))
}

#[tauri::command]
fn remove_tracks(state: State<'_, AppState>, ids: Vec<u64>) -> TrackList {
    let mut inner = state.0.lock().unwrap();
    let ids: HashSet<u64> = ids.into_iter().collect();
    inner.entries.retain(|e| !ids.contains(&e.id));
    list(&inner, 0)
}

#[tauri::command]
fn clear_tracks(state: State<'_, AppState>) -> TrackList {
    let mut inner = state.0.lock().unwrap();
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
    let (engine, cancel, work, overwrite) = {
        let mut inner = state.0.lock().unwrap();
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
                e.note = None;
                work.push((e.id as usize, e.track.query.clone()));
            }
        }
        inner.cancel.store(false, Ordering::Relaxed);
        (inner.engine.clone(), inner.cancel.clone(), work, overwrite)
    };
    let _ = overwrite;
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
        let mut prog = Progress { done: 0, total, found: 0, written: 0, missed: 0, running: true };
        let _ = app2.emit("progress", prog.clone());
        while let Some(ev) = rx.recv().await {
            if let Event::Challenge { provider, url } = &ev {
                let _ = app2.emit("challenge", serde_json::json!({ "provider": provider, "url": url }));
                continue;
            }
            let id = match &ev {
                Event::Trying { idx, .. }
                | Event::Skipped { idx, .. }
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
                        e.provider = Some(provider);
                    });
                }
                Event::Skipped { .. } | Event::Challenge { .. } => continue,
                Event::Cancelled { .. } => {
                    prog.done += 1;
                    set(&state, id, |e| e.status = Status::Idle);
                }
                Event::Failed { notes, .. } => {
                    prog.done += 1;
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
                    let (track, engine) = {
                        let inner = state.0.lock().unwrap();
                        (inner.entries.iter().find(|e| e.id == id).map(|e| e.track.clone()), inner.engine.clone())
                    };
                    let mut applied = None;
                    if auto_write {
                        if let Some(t) = track {
                            let l = lyrics.clone();
                            let r = tauri::async_runtime::spawn_blocking(move || engine.apply(&t, &l)).await;
                            applied = Some(r.map_err(err).and_then(|r| r.map_err(err)));
                        }
                    }
                    let write_tags = state.0.lock().unwrap().cfg.write_tags;
                    if let Some(Ok(_)) = &applied {
                        prog.written += 1;
                    }
                    set(&state, id, |e| {
                        e.provider = Some(lyrics.source.clone());
                        match &applied {
                            Some(Ok(_)) => {
                                e.status = Status::Written;
                                if write_tags {
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
            if let Some(d) = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto) {
                let _ = app2.emit("track", d);
            }
            let _ = app2.emit("progress", prog.clone());
        }
        prog.running = false;
        let _ = app2.emit("progress", prog);
        let _ = app2.emit("stats", list(&state.0.lock().unwrap(), 0).stats);
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
    let inner = state.0.lock().unwrap();
    let e = inner.entries.iter().find(|e| e.id == id)?;
    Some(LyricsView {
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

/// Write found lyrics for these tracks to disk (tags and/or sidecars, per settings).
#[tauri::command]
async fn apply_found(app: AppHandle, state: State<'_, AppState>, ids: Vec<u64>) -> CmdResult<usize> {
    let ids: HashSet<u64> = ids.into_iter().collect();
    let (engine, jobs, write_tags) = {
        let inner = state.0.lock().unwrap();
        let jobs: Vec<(u64, Track, Lyrics)> = inner
            .entries
            .iter()
            .filter(|e| ids.is_empty() || ids.contains(&e.id))
            .filter(|e| e.status == Status::Found)
            .filter_map(|e| e.found.clone().map(|l| (e.id, e.track.clone(), l)))
            .collect();
        (inner.engine.clone(), jobs, inner.cfg.write_tags)
    };
    let mut ok = 0;
    for (id, track, lyrics) in jobs {
        let eng = engine.clone();
        let l = lyrics.clone();
        let r = tauri::async_runtime::spawn_blocking(move || eng.apply(&track, &l)).await.map_err(err)?;
        set(&state, id, |e| match &r {
            Ok(_) => {
                e.status = Status::Written;
                if write_tags {
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
        if let Some(d) = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto) {
            let _ = app.emit("track", d);
        }
    }
    let _ = app.emit("stats", list(&state.0.lock().unwrap(), 0).stats);
    Ok(ok)
}

/// Save hand-edited lyrics into the file.
#[tauri::command]
async fn save_lyrics(app: AppHandle, state: State<'_, AppState>, id: u64, text: String) -> CmdResult<TrackDto> {
    let (path, keep) = {
        let inner = state.0.lock().unwrap();
        let e = inner.entries.iter().find(|e| e.id == id).context("unknown track").map_err(err)?;
        (e.track.path.clone(), inner.cfg.keep_mtime)
    };
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("lyrics are empty — use “Remove lyrics” to clear them".into());
    }
    let t = text.clone();
    tauri::async_runtime::spawn_blocking(move || tags::write_lyrics(&path, &t, keep)).await.map_err(err)?.map_err(err)?;
    set(&state, id, |e| {
        e.track.has_lyrics = true;
        e.embedded = Some(text);
        e.status = Status::Written;
        e.found = None;
        e.note = None;
    });
    let d = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto).context("unknown track").map_err(err)?;
    let _ = app.emit("track", d.clone());
    let _ = app.emit("stats", list(&state.0.lock().unwrap(), 0).stats);
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
        if let Some(d) = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto) {
            let _ = app.emit("track", d);
        }
    }
    let _ = app.emit("stats", list(&state.0.lock().unwrap(), 0).stats);
    Ok(n)
}

// ───────────────────────────── settings & plugins ─────────────────────────────

#[derive(Serialize)]
struct Settings {
    config: Config,
    providers: Vec<ProviderInfo>,
    engines: Vec<&'static str>,
    plugins_dir: String,
    config_path: String,
}

fn settings_of(cfg: &Config) -> Settings {
    Settings {
        config: cfg.clone(),
        providers: providers::catalog(&Config::plugins_dir()),
        engines: util::SEARCH_ENGINES.to_vec(),
        plugins_dir: Config::plugins_dir().display().to_string(),
        config_path: Config::path().map(|p| p.display().to_string()).unwrap_or_default(),
    }
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    settings_of(&state.0.lock().unwrap().cfg)
}

#[tauri::command]
fn set_settings(state: State<'_, AppState>, config: Config) -> CmdResult<Settings> {
    let engine = Engine::with_cookies(config.clone(), jar()).map_err(err)?;
    config.save().map_err(err)?;
    let mut inner = state.0.lock().unwrap();
    inner.cfg = config;
    inner.engine = engine;
    Ok(settings_of(&inner.cfg))
}

#[tauri::command]
fn install_plugin(state: State<'_, AppState>, dir: String) -> CmdResult<Settings> {
    use providers::plugin::ExternalProvider;
    let src = PathBuf::from(&dir);
    let loaded = ExternalProvider::load(&src).map_err(err)?;
    let dest = Config::plugins_dir().join(src.file_name().context("bad folder name").map_err(err)?);
    copy_dir(&src, &dest).map_err(err)?;
    let id = loaded.manifest().id.clone();
    let mut inner = state.0.lock().unwrap();
    if !inner.cfg.providers.contains(&id) {
        inner.cfg.providers.push(id);
        let cfg = inner.cfg.clone();
        cfg.save().map_err(err)?;
        inner.engine = Engine::with_cookies(cfg, jar()).map_err(err)?;
    }
    Ok(settings_of(&inner.cfg))
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        if e.file_name() == "__pycache__" {
            continue;
        }
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn health_check(state: State<'_, AppState>) -> CmdResult<()> {
    let engine = state.0.lock().unwrap().engine.clone();
    drop(engine);
    Ok(())
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
    let window = app.get_window("main").context("main window").map_err(err)?;
    let pos = LogicalPosition::new(rect.x, rect.y);
    let size = LogicalSize::new(rect.w.max(1.0), rect.h.max(1.0));
    state.0.lock().unwrap().browser_rect = Some(rect);
    if let Some(wv) = app.get_webview("browser") {
        wv.set_position(pos).map_err(err)?;
        wv.set_size(size).map_err(err)?;
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
    let mut builder = WebviewBuilder::new("browser", WebviewUrl::External(parse_url(&u)?));
    if mobile {
        builder = builder.user_agent(MOBILE_UA);
    }
    let wv = window.add_child(builder, pos, size).map_err(err)?;
    let _ = wv.set_zoom(zoom);
    state.0.lock().unwrap().browser_url = Some(u);
    Ok(())
}

/// A source asked for a bot check: show it in a real browser window. Cookies from that window are copied
/// into the engine's jar every second (so a check that passes by itself, or one the person solves, is
/// picked up), and the source's penalty is cleared when the window ends.
#[tauri::command]
async fn open_challenge(app: AppHandle, state: State<'_, AppState>, url: String, provider: String) -> CmdResult<()> {
    if let Some(w) = app.get_webview_window("challenge") {
        let _ = w.set_focus();
        return Ok(());
    }
    let u = parse_url(&url)?;
    let w = tauri::WebviewWindowBuilder::new(&app, "challenge", WebviewUrl::External(u.clone()))
        .title(format!("Verify you are human: {}", u.host_str().unwrap_or("")))
        .inner_size(560.0, 720.0)
        .user_agent(lyrixearch_core::providers::web::BROWSER_UA)
        .build()
        .map_err(err)?;
    let engine = state.0.lock().unwrap().engine.clone();
    let jar = jar();
    tauri::async_runtime::spawn(async move {
        for _ in 0..300 {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            let Ok(cookies) = w.cookies_for_url(u.clone()) else { break };
            let mut passed = false;
            for c in cookies {
                passed |= c.name() == "cf_clearance";
                jar.add_cookie_str(&c.to_string(), &u);
            }
            if passed {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                break;
            }
        }
        engine.clear_penalty(&provider);
        engine.forget_announced();
        let _ = w.app_handle().emit("challenge-done", provider);
        let _ = w.close();
    });
    Ok(())
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

#[tauri::command]
fn web_search_url(engine: String, artist: String, title: String) -> String {
    util::search_url(&engine, &artist, &title)
}

#[tauri::command]
fn open_external(url: String) -> CmdResult<()> {
    util::open_url(&url).map_err(err)
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
        (inner.entries.iter().filter(|e| ids.contains(&e.id)).map(|e| (e.id, e.track.path.clone(), e.track.has_lyrics)).collect::<Vec<_>>(), inner.cfg.clone())
    };
    let mut res = SidecarResult { embedded: 0, missing: 0, kept: 0, failed: 0 };
    for (id, path, has) in jobs {
        if has && !replace {
            res.kept += 1;
            continue;
        }
        let cfg = cfg.clone();
        let kind = kind.clone();
        let outcome = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Option<String>> {
            let Some((_, raw)) = providers::find_sidecar_kind(&path, kind.as_deref()) else { return Ok(None) };
            let plain = if lyrixearch_core::lrc::looks_synced(&raw) { lyrixearch_core::lrc::strip_timestamps(&raw) } else { raw };
            let text = lyrixearch_core::output::postprocess(&plain, &cfg);
            if text.trim().is_empty() {
                return Ok(None);
            }
            tags::write_lyrics(&path, &text, cfg.keep_mtime)?;
            Ok(Some(text))
        })
        .await
        .map_err(err)?;
        match outcome {
            Ok(Some(text)) => {
                res.embedded += 1;
                set(&state, id, |e| {
                    e.track.has_lyrics = true;
                    e.embedded = Some(text);
                    e.found = None;
                    e.provider = Some("file".into());
                    e.status = Status::Written;
                    e.note = None;
                });
            }
            Ok(None) => res.missing += 1,
            Err(m) => {
                res.failed += 1;
                set(&state, id, |e| {
                    e.status = Status::Error;
                    e.note = Some(format!("{m:#}"));
                });
            }
        }
        if let Some(d) = state.0.lock().unwrap().entries.iter().find(|e| e.id == id).map(dto) {
            let _ = app.emit("track", d);
        }
    }
    let _ = app.emit("stats", list(&state.0.lock().unwrap(), 0).stats);
    Ok(res)
}

#[derive(Serialize)]
struct ExportResult {
    written: usize,
    /// A file with that name already exists and replacing is off.
    kept: usize,
    /// No lyrics for the track, or no synced lyrics when `.lrc` was asked for.
    none: usize,
}

/// Write each track's lyrics (found or embedded) to a `.lrc` / `.txt` next to it.
/// `kind` is `auto` (`.lrc` when synced, else `.txt`), `lrc` or `txt`.
#[tauri::command]
fn export_lyrics(state: State<'_, AppState>, ids: Vec<u64>, kind: Option<String>, replace: Option<bool>) -> CmdResult<ExportResult> {
    let mode = match kind.as_deref() {
        Some("lrc") => lyrixearch_core::Sidecar::Lrc,
        Some("txt") => lyrixearch_core::Sidecar::Txt,
        _ => lyrixearch_core::Sidecar::Auto,
    };
    let replace = replace.unwrap_or(true);
    let inner = state.0.lock().unwrap();
    let ids: HashSet<u64> = ids.into_iter().collect();
    let mut res = ExportResult { written: 0, kept: 0, none: 0 };
    for e in inner.entries.iter().filter(|e| ids.contains(&e.id)) {
        let lyrics = match (&e.found, &e.embedded) {
            (Some(l), _) => l.clone(),
            (None, Some(text)) => Lyrics { plain: Some(text.clone()), ..Default::default() },
            _ => {
                res.none += 1;
                continue;
            }
        };
        match lyrixearch_core::output::sidecar_plan(&e.track.path, &lyrics, mode, &inner.cfg) {
            Some((path, body)) => {
                if path.exists() && !replace {
                    res.kept += 1;
                    continue;
                }
                std::fs::write(path, body).map_err(err)?;
                res.written += 1;
            }
            None => res.none += 1,
        }
    }
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
            app.get_webview_window("challenge").is_some()
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
  const out = {};
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const inv = (c, a) => window.__TAURI__.core.invoke(c, a || {});
  try {
    out.tauri = !!window.__TAURI__;
    out.mock = !!window.__MOCK__;
    await document.fonts.ready;
    out.fonts = document.fonts.check('15px "Source Sans 3"') && document.fonts.check('600 14px "Sora"') && document.fonts.check('600 14px "Inter"');
    out.layout = { inner: innerWidth, detail: Math.round(document.getElementById('detail').getBoundingClientRect().width), cssVar: getComputedStyle(document.getElementById('app')).getPropertyValue('--detail-w').trim(), saved: (() => { try { return localStorage.getItem('lf-detail-w'); } catch { return 'n/a'; } })() };
    const mark = document.querySelector('.mark');
    out.icon = mark.complete && mark.naturalWidth > 0;
    await sleep(1500);                         // the drop event was delivered by the backend
    const S = window.__lf.state;
    const tracks = [...S.tracks.values()];
    out.tracks = tracks.length;
    out.rows = document.querySelectorAll('.row').length;
    const by = (re) => tracks.find((t) => re.test(t.name));

    // search through the real command, with the camelCase argument the UI sends
    const plain = by(/plain/i) || tracks[0];
    await inv('search', { ids: [plain.id], autoWrite: false });
    for (let i = 0; i < 80; i++) { await sleep(500); const p = S.progress; if (p && !p.running && p.total > 0) break; }
    out.search = { status: S.tracks.get(plain.id).status, via: S.tracks.get(plain.id).provider, done: S.progress && S.progress.done };

    // cover thumbnail (embedded art)
    const cov = by(/cover/i);
    if (cov) { const c = await inv('get_cover', { id: cov.id }); out.cover = c ? c.slice(0, 23) + ' len=' + c.length : null; }

    // embed from a matching .lrc next to the track
    const sc = by(/sidecar/i);
    if (sc) { out.sidecar = await inv('embed_sidecar', { ids: [sc.id] }); out.sidecarState = S.tracks.get(sc.id).has_lyrics; }

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

    // the inspector, then the embedded browser
    window.__lf.selectId(plain.id);
    await sleep(500);
    document.querySelector('[data-tab=web]').click();
    await sleep(4000);
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
  } catch (e) { out.error = String((e && e.message) || e); }
  try {
    // verification window: a page that sets cf_clearance should be picked up and the window closed by itself
    await inv('open_challenge', { url: 'https://httpbin.org/cookies/set?cf_clearance=selftest', provider: 'selftest' });
    await sleep(6000);
  } catch (e) { out.challengeErr = String(e); }
  try { await window.__TAURI__.core.invoke('selftest_report', { info: JSON.stringify(out) }); } catch (e) { document.title = 'selftest failed: ' + e; }
})();
"#;

fn main() {
    // WebKitGTK's DMA-BUF renderer fails on many Wayland/Mesa/NVIDIA setups with "Error 71 (Protocol error)";
    // switching it off is harmless elsewhere. A user's own setting wins.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }
    let cfg = Config::load().unwrap_or_default();
    let engine = Engine::with_cookies(cfg.clone(), jar()).unwrap_or_else(|_| Engine::with_cookies(Config::default(), jar()).expect("default config is valid"));
    let state = AppState(Mutex::new(Inner {
        cfg,
        engine,
        entries: Vec::new(),
        next_id: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        browser_url: None,
        covers: HashMap::new(),
        pending_open: opened_from_args(),
        zoom: 1.0,
        browser_mobile: true,
        browser_rect: None,
    }));

    tauri::Builder::default()
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
            health_check,
            test_providers,
            browser_show,
            browser_hide,
            browser_back,
            browser_forward,
            browser_reload,
            web_search_url,
            open_external,
            selftest_report,
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
        ])
        .setup(|app| {
            app.manage(player::Player::start(app.handle().clone()));
            let window = WindowBuilder::new(app, "main")
                .title("Lyrixearch")
                .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))?)?
                .inner_size(1380.0, 880.0)
                .min_inner_size(980.0, 620.0)
                .build()?;
            let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
            window.add_child(WebviewBuilder::new("ui", WebviewUrl::App("index.html".into())), LogicalPosition::new(0.0, 0.0), size)?;

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
                                let _ = h.emit("tauri://drag-drop", serde_json::json!({ "paths": [dir], "position": { "x": 0, "y": 0 } }));
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
                        std::thread::sleep(std::time::Duration::from_secs(90));
                        eprintln!("SELFTEST timed out waiting for the UI");
                        h.exit(2);
                    });
                }
            }

            // The UI webview always fills the window; the browser panel is positioned by the UI.
            let handle = app.handle().clone();
            window.on_window_event(move |ev| {
                if let WindowEvent::Resized(_) = ev {
                    if let (Some(w), Some(ui)) = (handle.get_window("main"), handle.get_webview("ui")) {
                        if let (Ok(s), Ok(f)) = (w.inner_size(), w.scale_factor()) {
                            let _ = ui.set_size(s.to_logical::<f64>(f));
                        }
                        // macOS pins child webviews to the window's bottom edge, so a height change slides the
                        // browser panel away from its slot. Put it back where the UI last placed it.
                        if let (Some(wv), Some(r)) = (handle.get_webview("browser"), handle.state::<AppState>().0.lock().unwrap().browser_rect) {
                            let _ = wv.set_position(LogicalPosition::new(r.x, r.y));
                            let _ = wv.set_size(LogicalSize::new(r.w.max(1.0), r.h.max(1.0)));
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
            #[cfg(any(target_os = "macos", target_os = "ios"))]
            if let tauri::RunEvent::Opened { urls } = &event {
                let paths: Vec<String> = urls.iter().filter_map(|u| u.to_file_path().ok()).map(|p| p.to_string_lossy().into_owned()).collect();
                if !paths.is_empty() {
                    eprintln!("OPENED {paths:?}");
                    app.state::<AppState>().0.lock().unwrap().pending_open.extend(paths);
                    let _ = app.emit("open-paths", ());
                }
            }
            #[cfg(not(any(target_os = "macos", target_os = "ios")))]
            let _ = (app, event);
        });
}
