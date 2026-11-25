mod picker;
mod tui;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use lyrixearch_core::{scan, tags, Config, Engine, Event, Sidecar, Track};
use serde_json::json;
use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "lyrixearch", version, about = "Find lyrics for your music and embed them in the files")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Interactive terminal UI (default when no subcommand is given)
    Tui {
        /// Files or folders to load
        paths: Vec<PathBuf>,
    },
    /// List audio files with their tags and whether they already have lyrics
    Scan(ScanArgs),
    /// Search providers and write the lyrics found
    Fetch(FetchArgs),
    /// Embed the .lrc / .txt that sits next to each track (same name, same folder or a Lyrics folder)
    EmbedFiles {
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        /// Replace lyrics that are already embedded
        #[arg(long)]
        overwrite: bool,
        /// Report what would be embedded, change nothing
        #[arg(long)]
        dry_run: bool,
        /// One JSON object per track on stdout
        #[arg(long)]
        json: bool,
    },
    /// Print the lyrics embedded in a file
    Show { path: PathBuf },
    /// Embed lyrics you supply: from a .lrc / .txt file, or from stdin when no --from is given
    Set {
        /// The audio file
        path: PathBuf,
        /// Lyrics file to read (default: stdin)
        #[arg(long)]
        from: Option<PathBuf>,
    },
    /// Save embedded lyrics to a .lrc / .txt next to each track
    Export {
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        /// auto (.lrc when the lyrics are timed, else .txt) | lrc | txt
        #[arg(long, default_value = "auto")]
        format: Sidecar,
        /// Replace .lrc / .txt files that already exist
        #[arg(long)]
        overwrite: bool,
        /// One JSON object per track on stdout
        #[arg(long)]
        json: bool,
    },
    /// Remove embedded lyrics from files
    Remove {
        paths: Vec<PathBuf>,
        #[arg(short, long)]
        recursive: bool,
        /// Do not ask for confirmation
        #[arg(short, long)]
        yes: bool,
        /// One JSON object per track on stdout
        #[arg(long)]
        json: bool,
    },
    /// List providers, or health-check them
    Providers {
        /// Query every provider once and report status and latency
        #[arg(long)]
        test: bool,
        /// One JSON object per provider on stdout
        #[arg(long)]
        json: bool,
    },
    /// Manage plugins (external scripts that act as extra lyrics sources)
    Plugins {
        #[command(subcommand)]
        action: PluginCmd,
    },
    /// View or change settings
    Config {
        #[command(subcommand)]
        action: ConfigCmd,
    },
}

#[derive(Subcommand)]
enum PluginCmd {
    /// List installed plugins and any that failed to load
    List {
        /// One JSON object per installed plugin on stdout
        #[arg(long)]
        json: bool,
    },
    /// Print the plugins folder
    Path,
    /// Copy a plugin folder (containing plugin.toml) into the plugins folder and enable it
    Install { dir: PathBuf },
    /// Use an installed plugin as a source (added at the end of `providers`)
    Enable { id: String },
    /// Stop using a plugin as a source (it stays installed)
    Disable { id: String },
    /// Run one plugin by hand and show what it returns (lyrics text is not printed)
    Test {
        id: String,
        #[arg(long)]
        artist: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        album: Option<String>,
        #[arg(long)]
        track: Option<u32>,
    },
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Print current settings as TOML
    Show,
    /// Print the settings file path
    Path,
    /// Print one setting
    Get { key: String },
    /// Set one key, e.g. `config set sidecar auto`
    Set { key: String, value: String },
    /// Restore defaults
    Reset,
}

#[derive(Args)]
struct ScanArgs {
    paths: Vec<PathBuf>,
    #[arg(short, long)]
    recursive: bool,
    #[arg(long)]
    json: bool,
}

#[derive(Args)]
struct FetchArgs {
    /// Files or folders
    paths: Vec<PathBuf>,
    #[arg(short, long)]
    recursive: bool,
    /// Comma-separated provider ids, in order (see `providers`)
    #[arg(long, value_delimiter = ',')]
    providers: Option<Vec<String>>,
    /// Tracks searched at once
    #[arg(short = 'j', long)]
    concurrency: Option<usize>,
    /// Sidecar files: off | auto | lrc | txt
    #[arg(long)]
    sidecar: Option<Sidecar>,
    /// Do not write lyrics into the audio files
    #[arg(long)]
    no_tags: bool,
    /// Replace lyrics that are already embedded
    #[arg(long)]
    overwrite: bool,
    /// Accept only synced lyrics
    #[arg(long)]
    synced: bool,
    /// Preserve file modification times
    #[arg(long, conflicts_with = "no_keep_mtime")]
    keep_mtime: bool,
    /// Let the modification time change when writing lyrics
    #[arg(long)]
    no_keep_mtime: bool,
    /// Search and report, but write nothing
    #[arg(long)]
    dry_run: bool,
    /// Also write AI transcriptions (normally they are reported, for checking first)
    #[arg(long)]
    write_transcriptions: bool,
    /// One JSON object per track on stdout
    #[arg(long)]
    json: bool,
    /// Also print which provider is being tried
    #[arg(short, long)]
    verbose: bool,
}

fn load_tracks(paths: &[PathBuf], recursive: bool) -> (Vec<Track>, Vec<(PathBuf, String)>) {
    let files = scan::collect_audio(paths, recursive);
    let mut tracks = Vec::new();
    let mut errors = Vec::new();
    for f in files {
        match tags::read_track(&f) {
            Ok(t) => tracks.push(t),
            Err(e) => errors.push((f, format!("{e:#}"))),
        }
    }
    (tracks, errors)
}

fn main() -> Result<ExitCode> {
    let cli = Cli::parse();
    let cfg = Config::load()?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;

    match cli.cmd {
        None => tui::run(rt.handle().clone(), Vec::new(), cfg).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Tui { paths }) => tui::run(rt.handle().clone(), paths, cfg).map(|_| ExitCode::SUCCESS),
        Some(Cmd::Scan(a)) => scan_cmd(a),
        Some(Cmd::Fetch(a)) => rt.block_on(fetch_cmd(a, cfg)),
        Some(Cmd::Show { path }) => match tags::read_lyrics(&path) {
            Some(t) => {
                println!("{t}");
                Ok(ExitCode::SUCCESS)
            }
            None => {
                eprintln!("no embedded lyrics in {}", path.display());
                Ok(ExitCode::FAILURE)
            }
        },
        Some(Cmd::EmbedFiles { paths, recursive, overwrite, dry_run, json }) => embed_files_cmd(paths, recursive, overwrite, dry_run, json, &cfg),
        Some(Cmd::Set { path, from }) => set_cmd(path, from, &cfg),
        Some(Cmd::Export { paths, recursive, format, overwrite, json }) => export_cmd(paths, recursive, format, overwrite, json, &cfg),
        Some(Cmd::Remove { paths, recursive, yes, json }) => remove_cmd(paths, recursive, yes, json, &cfg),
        Some(Cmd::Providers { test, json }) => rt.block_on(providers_cmd(test, json, cfg)),
        Some(Cmd::Plugins { action }) => rt.block_on(plugins_cmd(action, cfg)),
        Some(Cmd::Config { action }) => config_cmd(action, cfg),
    }
}

fn scan_cmd(a: ScanArgs) -> Result<ExitCode> {
    let (tracks, errors) = load_tracks(&a.paths, a.recursive);
    if a.json {
        for t in &tracks {
            println!("{}", json!({"path": t.path, "artist": t.query.artist, "title": t.query.title, "album": t.query.album, "has_lyrics": t.has_lyrics}));
        }
    } else {
        for t in &tracks {
            println!("{} {} — {}   {}", if t.has_lyrics { "♪" } else { " " }, t.query.artist, t.query.title, t.path.display());
        }
        let with = tracks.iter().filter(|t| t.has_lyrics).count();
        println!("\n{} tracks, {} with lyrics, {} without", tracks.len(), with, tracks.len() - with);
    }
    for (p, e) in &errors {
        eprintln!("skipped {}: {e}", p.display());
    }
    Ok(ExitCode::SUCCESS)
}

async fn fetch_cmd(a: FetchArgs, mut cfg: Config) -> Result<ExitCode> {
    if a.paths.is_empty() {
        bail!("give at least one file or folder");
    }
    if let Some(p) = a.providers {
        cfg.providers = p;
    }
    if let Some(c) = a.concurrency {
        cfg.concurrency = c.max(1);
    }
    if let Some(s) = a.sidecar {
        cfg.sidecar = s;
    }
    if a.no_tags {
        cfg.write_tags = false;
    }
    if a.overwrite {
        cfg.overwrite = true;
    }
    if a.synced {
        cfg.synced_only = true;
    }
    if a.keep_mtime || a.no_keep_mtime {
        cfg.keep_mtime = a.keep_mtime;
    }
    let engine = Engine::new(cfg.clone())?;

    let (tracks, errors) = load_tracks(&a.paths, a.recursive);
    for (p, e) in &errors {
        eprintln!("skipped {}: {e}", p.display());
    }
    let work: Vec<(usize, &Track)> = tracks.iter().enumerate().filter(|(_, t)| cfg.overwrite || !t.has_lyrics).collect();
    let skipped = tracks.len() - work.len();
    if !a.json {
        eprintln!("{} tracks to search ({} already have lyrics){}", work.len(), skipped, if a.dry_run { " — dry run" } else { "" });
    }

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.store(true, Ordering::Relaxed);
                eprintln!("\nstopping after in-flight searches…");
            }
        });
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let items: Vec<(usize, lyrixearch_core::Query)> = work.iter().map(|(i, t)| (*i, t.query.clone())).collect();
    let runner = {
        let engine = engine.clone();
        let cancel = cancel.clone();
        tokio::spawn(async move { engine.run(items, tx, cancel).await })
    };

    let (mut found, mut missing, mut failed, mut unanswered, mut done) = (0usize, 0usize, 0usize, 0usize, 0usize);
    let total = work.len();
    while let Some(ev) = rx.recv().await {
        match ev {
            Event::Trying { idx, provider } => {
                if a.verbose && !a.json {
                    eprintln!("  … {} via {provider}", tracks[idx].display_name());
                }
            }
            Event::Skipped { idx, provider, reason } => {
                if a.verbose && !a.json {
                    eprintln!("  ⏭ {}: {provider} skipped ({reason})", tracks[idx].display_name());
                }
            }
            Event::Tried { idx, provider, result, millis } => {
                if a.verbose && !a.json && result != "found" {
                    eprintln!("  · {}: {provider}: {result} ({millis} ms)", tracks[idx].display_name());
                }
            }
            Event::Cancelled { .. } => {}
            Event::Challenge { provider, .. } => {
                if !a.json {
                    eprintln!("  ! {provider} wants a bot check passed; the desktop app opens a window for that");
                }
            }
            Event::Failed { idx, notes } => {
                done += 1;
                unanswered += 1;
                let t = &tracks[idx];
                if a.json {
                    println!("{}", json!({"path": t.path, "status": "failed", "notes": notes}));
                } else {
                    eprintln!("[{done}/{total}] ! {} — {} (no source answered; try again later)", t.query.artist, t.query.title);
                }
            }
            Event::NotFound { idx, notes } => {
                done += 1;
                missing += 1;
                let t = &tracks[idx];
                if a.json {
                    println!("{}", json!({"path": t.path, "status": "not_found", "notes": notes}));
                } else {
                    eprintln!("[{done}/{total}] ✗ {} — {}", t.query.artist, t.query.title);
                }
            }
            Event::Found { idx, lyrics } => {
                done += 1;
                let t = &tracks[idx];
                let held = lyrics.review && !a.write_transcriptions;
                if held && !a.json {
                    eprintln!("  · {}: transcribed by AI, not written: check it, then `lyrixearch set`, or use --write-transcriptions", t.display_name());
                }
                let applied = if a.dry_run || held { Ok(Default::default()) } else { engine.apply(t, &lyrics) };
                match applied {
                    Ok(ap) => {
                        found += 1;
                        if a.json {
                            println!(
                                "{}",
                                json!({"path": t.path, "status": "found", "source": lyrics.source, "score": lyrics.score,
                                       "matched": format!("{} - {}", lyrics.matched_artist, lyrics.matched_title),
                                       "synced": lyrics.synced.is_some(), "tag_written": ap.tag_written, "sidecar": ap.sidecar})
                            );
                        } else {
                            eprintln!(
                                "[{done}/{total}] ✓ {} — {}  ← {} ({:.0}%){}{}",
                                t.query.artist,
                                t.query.title,
                                lyrics.source,
                                lyrics.score * 100.0,
                                if lyrics.synced.is_some() { " synced" } else { "" },
                                ap.sidecar.map(|p| format!(" +{}", p.file_name().unwrap_or_default().to_string_lossy())).unwrap_or_default()
                            );
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        if a.json {
                            println!("{}", json!({"path": t.path, "status": "error", "error": format!("{e:#}")}));
                        } else {
                            eprintln!("[{done}/{total}] ! {} — {}: {e:#}", t.query.artist, t.query.title);
                        }
                    }
                }
            }
        }
    }
    runner.await.context("search task panicked")?;
    if !a.json {
        eprintln!("\nfound {found}, no match {missing}, failed (no source answered) {unanswered}, write errors {failed}, already had lyrics {skipped}");
    }
    Ok(if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn embed_files_cmd(paths: Vec<PathBuf>, recursive: bool, overwrite: bool, dry_run: bool, json: bool, cfg: &Config) -> Result<ExitCode> {
    use lyrixearch_core::{output, SidecarEmbed};
    if paths.is_empty() {
        bail!("give at least one file or folder");
    }
    let (tracks, errors) = load_tracks(&paths, recursive);
    for (p, e) in &errors {
        eprintln!("skipped {}: {e}", p.display());
    }
    let (mut done, mut missing, mut skipped, mut failed) = (0usize, 0usize, 0usize, 0usize);
    for t in &tracks {
        let path = t.path.to_string_lossy();
        match output::embed_from_sidecar(t, cfg, overwrite, None, dry_run) {
            Ok(SidecarEmbed::AlreadyHad) => {
                skipped += 1;
                if json {
                    println!("{}", json!({"path": path, "status": "already_had"}));
                }
            }
            Ok(SidecarEmbed::NoFile) => {
                missing += 1;
                if json {
                    println!("{}", json!({"path": path, "status": "no_file"}));
                }
            }
            Ok(SidecarEmbed::WouldEmbed(file)) => {
                done += 1;
                if json {
                    println!("{}", json!({"path": path, "status": "would_embed", "sidecar": file.to_string_lossy()}));
                } else {
                    println!("would embed {} ← {}", t.path.display(), file.display());
                }
            }
            Ok(SidecarEmbed::Embedded(file)) => {
                done += 1;
                if json {
                    println!("{}", json!({"path": path, "status": "embedded", "sidecar": file.to_string_lossy()}));
                } else {
                    println!("embedded {} ← {}", t.path.display(), file.file_name().unwrap_or_default().to_string_lossy());
                }
            }
            Err(e) => {
                failed += 1;
                if json {
                    println!("{}", json!({"path": path, "status": "error", "error": format!("{e:#}")}));
                } else {
                    eprintln!("failed {}: {e:#}", t.path.display());
                }
            }
        }
    }
    if !json {
        eprintln!(
            "\n{} {done}, no matching file {missing}, already had lyrics {skipped}{}",
            if dry_run { "would embed" } else { "embedded" },
            if failed > 0 { format!(", failed {failed}") } else { String::new() }
        );
    }
    Ok(if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn set_cmd(path: PathBuf, from: Option<PathBuf>, cfg: &Config) -> Result<ExitCode> {
    use std::io::Read;
    if !path.is_file() {
        bail!("{} is not a file", path.display());
    }
    let mut text = String::new();
    match &from {
        Some(f) => text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?,
        None => {
            if std::io::stdin().is_terminal() {
                eprintln!("paste the lyrics, then press Ctrl-D");
            }
            std::io::stdin().read_to_string(&mut text)?;
        }
    }
    let text = text.trim();
    if text.is_empty() {
        bail!("no lyrics given (use `remove` to clear a track's lyrics)");
    }
    // timed lyrics keep their timing: plain text in the lyrics tag, the LRC alongside where the format has room
    if lyrixearch_core::lrc::looks_synced(text) {
        let plain = lyrixearch_core::lrc::strip_timestamps(text);
        tags::write_lyrics_with(&path, plain.trim(), Some(text), cfg.keep_mtime)?;
    } else {
        tags::write_lyrics(&path, text, cfg.keep_mtime)?;
    }
    println!("embedded: {}", path.display());
    Ok(ExitCode::SUCCESS)
}

fn export_cmd(paths: Vec<PathBuf>, recursive: bool, format: Sidecar, overwrite: bool, json: bool, cfg: &Config) -> Result<ExitCode> {
    use lyrixearch_core::Lyrics;
    let format = if format == Sidecar::Off { Sidecar::Auto } else { format };
    let (tracks, _) = load_tracks(&paths, recursive);
    let (mut written, mut kept, mut none, mut failed) = (0, 0, 0, 0);
    for t in &tracks {
        let report = |status: &str, file: Option<&std::path::Path>, error: Option<String>| {
            if json {
                println!("{}", json!({"path": t.path.to_string_lossy(), "status": status, "file": file.map(|f| f.to_string_lossy()), "error": error}));
            }
        };
        let Some(text) = tags::read_lyrics(&t.path) else {
            none += 1;
            report("no_lyrics", None, None);
            continue;
        };
        // the timed copy (ID3 SYLT / Vorbis SYNCEDLYRICS), or LRC text stored in the lyrics tag itself
        let synced = tags::read_synced(&t.path).or_else(|| lyrixearch_core::lrc::looks_synced(&text).then(|| text.clone()));
        let lyrics = Lyrics { plain: Some(text), synced, ..Default::default() };
        let Some((file, body)) = lyrixearch_core::output::sidecar_plan(&t.path, &lyrics, format, cfg) else {
            // asked for .lrc but the lyrics have no timing
            none += 1;
            report("not_timed", None, None);
            continue;
        };
        if file.exists() && !overwrite {
            kept += 1;
            report("exists", Some(&file), None);
            if !json {
                println!("kept (exists): {}", file.display());
            }
            continue;
        }
        match lyrixearch_core::atomic::write(&file, body.as_bytes()) {
            Ok(()) => {
                written += 1;
                report("written", Some(&file), None);
                if !json {
                    println!("wrote: {}", file.display());
                }
            }
            Err(e) => {
                failed += 1;
                report("error", Some(&file), Some(format!("{e:#}")));
                if !json {
                    eprintln!("failed: {}: {e:#}", file.display());
                }
            }
        }
    }
    if !json {
        println!("\n{written} written, {kept} kept (use --overwrite to replace), {none} without usable lyrics, {failed} failed");
    }
    Ok(if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn remove_cmd(paths: Vec<PathBuf>, recursive: bool, yes: bool, json: bool, cfg: &Config) -> Result<ExitCode> {
    if !yes && !std::io::stdin().is_terminal() {
        bail!("remove requires --yes when stdin is not a terminal");
    }
    let (tracks, _) = load_tracks(&paths, recursive);
    let targets: Vec<&Track> = tracks.iter().filter(|t| t.has_lyrics).collect();
    if targets.is_empty() {
        if !json {
            println!("nothing to remove");
        }
        return Ok(ExitCode::SUCCESS);
    }
    if !yes {
        eprint!("remove embedded lyrics from {} file(s)? [y/N] ", targets.len());
        let mut s = String::new();
        std::io::stdin().read_line(&mut s)?;
        if !s.trim().eq_ignore_ascii_case("y") {
            return Ok(ExitCode::FAILURE);
        }
    }
    let mut failed = 0;
    for t in targets {
        match tags::remove_lyrics(&t.path, cfg.keep_mtime) {
            Ok(()) if json => println!("{}", json!({"path": t.path.to_string_lossy(), "status": "removed"})),
            Ok(()) => println!("removed: {}", t.path.display()),
            Err(e) => {
                failed += 1;
                if json {
                    println!("{}", json!({"path": t.path.to_string_lossy(), "status": "error", "error": format!("{e:#}")}));
                } else {
                    eprintln!("failed: {}: {e:#}", t.path.display());
                }
            }
        }
    }
    Ok(if failed > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

async fn providers_cmd(test: bool, json: bool, cfg: Config) -> Result<ExitCode> {
    let all: Vec<String> = lyrixearch_core::providers::all(&Config::plugins_dir()).iter().map(|p| p.id().to_string()).collect();
    if !test {
        if !json {
            println!("order  provider");
        }
        for (i, id) in cfg.providers.iter().enumerate() {
            if json {
                println!("{}", json!({"id": id, "enabled": true, "order": i + 1}));
            } else {
                println!("{:>5}  {id}", i + 1);
            }
        }
        for id in all.iter().filter(|id| !cfg.providers.iter().any(|p| &p == id)) {
            if json {
                println!("{}", json!({"id": id, "enabled": false, "order": null}));
            } else {
                println!("  off  {id}");
            }
        }
        return Ok(ExitCode::SUCCESS);
    }
    let mut c = cfg;
    c.providers = all;
    let engine = Engine::new(c)?;
    let mut bad = 0;
    for (id, res, took) in engine.test_providers().await {
        if json {
            let (status, detail) = match &res {
                Ok(true) => ("ok", None),
                Ok(false) => ("no_result", Some("reachable, no result for the test track")),
                Err(e) => ("error", Some(e.as_str())),
            };
            bad += usize::from(status != "ok");
            println!("{}", json!({"id": id, "status": status, "detail": detail, "millis": took.as_millis()}));
            continue;
        }
        match res {
            Ok(true) => println!("✓ {id:<12} {took:>6.0?}"),
            Ok(false) => {
                bad += 1;
                println!("· {id:<12} {took:>6.0?}  reachable, no result for the test track");
            }
            Err(e) => {
                bad += 1;
                println!("✗ {id:<12} {took:>6.0?}  {e}");
            }
        }
    }
    Ok(if bad > 0 { ExitCode::FAILURE } else { ExitCode::SUCCESS })
}

fn config_cmd(action: ConfigCmd, mut cfg: Config) -> Result<ExitCode> {
    match action {
        ConfigCmd::Show => print!("{}", toml::to_string_pretty(&cfg)?),
        ConfigCmd::Path => println!("{}", Config::path().map(|p| p.display().to_string()).unwrap_or_else(|| "(none)".into())),
        ConfigCmd::Get { key } => println!("{}", cfg.get(&key)?),
        ConfigCmd::Set { key, value } => {
            cfg.set(&key, &value)?;
            println!("saved to {}", cfg.save()?.display());
        }
        ConfigCmd::Reset => println!("saved to {}", Config::default().save()?.display()),
    }
    Ok(ExitCode::SUCCESS)
}

fn set_plugin_enabled(id: &str, on: bool, mut cfg: Config) -> Result<()> {
    let has = cfg.providers.iter().any(|p| p == id);
    match (on, has) {
        (true, true) => println!("`{id}` is already enabled"),
        (false, false) => println!("`{id}` is not enabled"),
        (true, false) => {
            cfg.providers.push(id.to_string());
            cfg.save()?;
            println!("enabled `{id}` (tried after the other sources; reorder with `lyrixearch config set providers …`)");
        }
        (false, true) => {
            cfg.providers.retain(|p| p != id);
            cfg.save()?;
            println!("disabled `{id}`");
        }
    }
    Ok(())
}

async fn plugins_cmd(action: PluginCmd, cfg: Config) -> Result<ExitCode> {
    use lyrixearch_core::providers::plugin;
    let dir = Config::plugins_dir();
    match action {
        PluginCmd::Path => println!("{}", dir.display()),
        PluginCmd::List { json } => {
            let (ok, bad) = plugin::discover(&dir);
            if !json && ok.is_empty() && bad.is_empty() {
                println!("no plugins installed in {}", dir.display());
            }
            for p in &ok {
                let m = p.manifest();
                let enabled = cfg.providers.contains(&m.id);
                if json {
                    println!("{}", json!({"id": m.id, "name": m.name, "path": p.dir().to_string_lossy(), "enabled": enabled, "status": "loaded"}));
                    continue;
                }
                let enabled = if enabled { "enabled" } else { "not in `providers` — add it with: lyrixearch config set providers …" };
                println!("{:<14} {}  [{}]\n               {}", m.id, m.name, enabled, p.dir().display());
            }
            for (path, e) in &bad {
                if json {
                    println!("{}", json!({"path": path.to_string_lossy(), "status": "error", "error": format!("{e:#}")}));
                } else {
                    println!("✗ {}: {e:#}", path.display());
                }
            }
        }
        PluginCmd::Install { dir: src } => {
            if plugin::ExternalProvider::load(&src).is_ok_and(|p| p.manifest().build.is_some()) {
                eprintln!("building the plugin…");
            }
            let loaded = plugin::ExternalProvider::install(&src, &dir)?;
            let id = loaded.manifest().id.clone();
            println!("installed `{id}` to {}", dir.join(src.file_name().unwrap_or_default()).display());
            // same as the GUI: an installed plugin is used straight away
            set_plugin_enabled(&id, true, cfg)?;
        }
        PluginCmd::Enable { id } => {
            let (ok, _) = plugin::discover(&dir);
            if !ok.iter().any(|p| p.manifest().id == id) {
                bail!("no plugin `{id}` in {} (see `lyrixearch plugins list`)", dir.display());
            }
            set_plugin_enabled(&id, true, cfg)?;
        }
        PluginCmd::Disable { id } => set_plugin_enabled(&id, false, cfg)?,
        PluginCmd::Test { id, artist, title, album, track } => {
            let (ok, _) = plugin::discover(&dir);
            let p = ok.into_iter().find(|p| p.manifest().id == id).with_context(|| format!("no plugin `{id}` in {}", dir.display()))?;
            use lyrixearch_core::providers::Provider;
            let q = lyrixearch_core::Query { artist, title, album, track_number: track, ..Default::default() };
            let started = std::time::Instant::now();
            match p.fetch(&reqwest_client(), &q, &cfg).await? {
                Some(l) => println!(
                    "found in {:.1?}: {} — {}  plain={} chars synced={} chars score={:.2}",
                    started.elapsed(),
                    l.matched_artist,
                    l.matched_title,
                    l.plain.as_deref().map(str::len).unwrap_or(0),
                    l.synced.as_deref().map(str::len).unwrap_or(0),
                    l.score
                ),
                None => println!("no result ({:.1?})", started.elapsed()),
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn reqwest_client() -> lyrixearch_core::HttpClient {
    lyrixearch_core::HttpClient::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fetch_accepts_either_mtime_override() {
        for (flag, expected) in [("--keep-mtime", true), ("--no-keep-mtime", false)] {
            let cli = Cli::try_parse_from(["lyrixearch", "fetch", "--synced", flag, "song.flac"]).unwrap();
            let Some(Cmd::Fetch(args)) = cli.cmd else { panic!("expected fetch") };
            assert!(args.synced);
            assert_eq!(args.keep_mtime, expected);
            assert_eq!(args.no_keep_mtime, !expected);
        }
        assert!(Cli::try_parse_from(["lyrixearch", "fetch", "--keep-mtime", "--no-keep-mtime"]).is_err());
    }
}
