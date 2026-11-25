use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Sidecar {
    /// Never write sidecar files.
    Off,
    /// `.lrc` when synced lyrics exist, otherwise `.txt`.
    #[default]
    Auto,
    /// Only `.lrc` (and only when synced lyrics exist).
    Lrc,
    /// Always `.txt`.
    Txt,
}

impl std::str::FromStr for Sidecar {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_ascii_lowercase().as_str() {
            "off" | "none" => Ok(Self::Off),
            "auto" => Ok(Self::Auto),
            "lrc" => Ok(Self::Lrc),
            "txt" => Ok(Self::Txt),
            other => Err(format!("unknown sidecar mode `{other}` (off|auto|lrc|txt)")),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Provider ids in the order they are tried for each track.
    pub providers: Vec<String>,
    /// Tracks searched at the same time.
    pub concurrency: usize,
    /// Per-provider request budget for one track.
    pub provider_timeout_secs: u64,
    /// A provider that fails this many times within the window is skipped for the cooldown.
    pub penalty_max_failures: usize,
    pub penalty_window_secs: u64,
    pub penalty_cooldown_secs: u64,
    pub write_tags: bool,
    pub sidecar: Sidecar,
    /// Restore the file's modified time after tagging.
    pub keep_mtime: bool,
    /// Replace lyrics that are already embedded.
    pub overwrite: bool,
    /// Drop `[Verse]` / `[Chorus]` style header lines.
    pub remove_section_tags: bool,
    /// Insert a blank first line (some players want it).
    pub prepend_newline: bool,
    /// Optional proxy URL, e.g. `http://127.0.0.1:8080` or `socks5://...`.
    pub proxy: Option<String>,
    /// Minimum overall match score to accept a candidate (0..1).
    pub min_score: f32,
    /// Engine used by "search the web" in the GUI/TUI: google | google_ai | brave_ai | bing | duckduckgo | yahoo | yandex.
    pub search_engine: String,
    /// Folders the GUI re-scans for tracks without lyrics.
    pub library_dirs: Vec<String>,
    /// Starting points offered by the terminal UI's folder picker (for example `/Volumes/Music/Audio`).
    pub browse_roots: Vec<String>,
    /// What the terminal UI added last, so "run last again" can repeat it.
    pub last_added: Vec<String>,
    /// Terminal UI: start searching the tracks that were just added.
    pub tui_search_new: bool,
    /// Accept only results that come with synced (timestamped) lyrics.
    pub synced_only: bool,
    /// Optional Last.fm API key. When set, artist and title are run through Last.fm's
    /// correction service before searching (fixes misspellings and alternate names).
    pub lastfm_api_key: Option<String>,
    /// Strip "(Remastered 2011)", "[Live at ...]" style noise from titles before searching.
    pub tag_clean_enabled: bool,
    /// Extra regex removed from titles before searching (applied after the built-in clean-up).
    pub tag_clean_pattern: Option<String>,
    /// Built-in sources this settings file has been offered. A source added in a later version is
    /// switched on once (appended to `providers`), so new sources reach existing setups.
    /// A file without this field predates it, so it knew the original sources.
    #[serde(default = "original_builtins")]
    pub known_providers: Vec<String>,
    /// Keep timing for single words (Enhanced LRC) where a source has it: in .lrc files and the
    /// SYNCEDLYRICS tag of FLAC / Ogg. MP3's SYLT frame always gets line timing.
    pub word_timing: bool,
    /// AI transcription (the `ai_transcribe` source): `local` (whisper.cpp), `groq` or `openai`.
    pub transcribe_engine: String,
    /// API key for `groq` / `openai` transcription.
    pub transcribe_api_key: Option<String>,
    /// Language hint for transcription (`en`, `de`, ...); empty or `auto` detects it.
    pub transcribe_language: Option<String>,
    /// whisper.cpp model file; empty uses the one downloaded in Settings.
    pub whisper_model: Option<String>,
    /// whisper.cpp program; empty finds `whisper-cli`.
    pub whisper_bin: Option<String>,
    /// Desktop app: search tracks as soon as they are added to the list.
    pub auto_fetch: bool,
    /// Desktop app: closing the window keeps the app running in the menu bar / tray.
    pub keep_running: bool,
    /// Desktop app: a system notification when a fetch finishes while the window is in the background.
    pub notify_done: bool,
    /// Stop the computer from sleeping while a fetch runs.
    pub keep_awake: bool,
    /// Desktop app, macOS: a drop zone opens under the menu bar icon while files are dragged near it.
    pub drop_shelf: bool,
}

fn original_builtins() -> Vec<String> {
    crate::providers::ORIGINAL_BUILTINS.iter().map(|s| s.to_string()).collect()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Same order the original ships with, restricted to sources this build implements.
            providers: crate::providers::default_ids(),
            concurrency: 10,
            provider_timeout_secs: 8,
            penalty_max_failures: 3,
            penalty_window_secs: 60,
            penalty_cooldown_secs: 300,
            write_tags: true,
            sidecar: Sidecar::Off,
            keep_mtime: true,
            overwrite: false,
            remove_section_tags: false,
            prepend_newline: false,
            proxy: None,
            min_score: 0.8,
            search_engine: "google".into(),
            library_dirs: Vec::new(),
            browse_roots: Vec::new(),
            last_added: Vec::new(),
            tui_search_new: true,
            synced_only: false,
            lastfm_api_key: None,
            tag_clean_enabled: true,
            tag_clean_pattern: None,
            known_providers: crate::providers::builtin_ids(),
            word_timing: false,
            transcribe_engine: "local".into(),
            transcribe_api_key: None,
            transcribe_language: None,
            whisper_model: None,
            whisper_bin: None,
            auto_fetch: false,
            keep_running: true,
            notify_done: true,
            keep_awake: true,
            drop_shelf: true,
        }
    }
}

impl Config {
    /// `<config>/lyrixearch`. The first time it is asked for, settings from an earlier name or location
    /// are moved here, so settings and installed plugins carry over.
    pub fn config_dir() -> Option<PathBuf> {
        let new = Self::xdg_base()?.join("lyrixearch");
        if !new.exists() {
            // carry settings over from earlier locations: the old app name, and macOS's Application Support
            let native = dirs::config_dir();
            let olds =
                [native.as_ref().map(|b| b.join("lyrixearch")), native.as_ref().map(|b| b.join("lyricfind")), Self::xdg_base().map(|b| b.join("lyricfind"))];
            if let Some(old) = olds.into_iter().flatten().find(|p| p.is_dir() && *p != new) {
                let moved = new.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::rename(&old, &new));
                if let Err(e) = moved {
                    // Moving failed (another volume, permissions): keep using the old folder so the
                    // user's settings and plugins are still found, rather than starting empty.
                    eprintln!("lyrixearch: could not move settings from {} to {}: {e}; using the old folder", old.display(), new.display());
                    return Some(old);
                }
            }
        }
        Some(new)
    }

    /// `$XDG_CONFIG_HOME` or `~/.config` on macOS and Linux (where terminal tools keep their settings);
    /// the platform folder (`%APPDATA%`) on Windows.
    fn xdg_base() -> Option<PathBuf> {
        if cfg!(windows) {
            return dirs::config_dir();
        }
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute()).or_else(|| dirs::home_dir().map(|h| h.join(".config")))
    }

    /// Folder scanned for plugins (`<config>/lyrixearch/plugins`).
    pub fn plugins_dir() -> PathBuf {
        Self::config_dir().map(|d| d.join("plugins")).unwrap_or_else(|| PathBuf::from("plugins"))
    }

    pub fn path() -> Option<PathBuf> {
        Self::config_dir().map(|d| d.join("config.toml"))
    }

    /// Load from the default location; missing file means defaults. A file that cannot be read
    /// or parsed is an error: callers must not save over it (see [`Config::quarantine_unreadable`]).
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else { return Ok(Self::default()) };
        Self::load_from(&path)
    }

    /// Load from `path`; missing file means defaults.
    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => {
                let mut cfg: Self = toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?;
                cfg.validate().with_context(|| format!("checking {}", path.display()))?;
                cfg.offer_new_providers();
                Ok(cfg)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Switch on built-in sources added since this settings file was written (they go last), and
    /// remember that every current source has now been offered.
    pub fn offer_new_providers(&mut self) {
        let removed = crate::providers::REMOVED_BUILTINS;
        self.providers.retain(|p| !removed.contains(&p.as_str()));
        self.known_providers.retain(|p| !removed.contains(&p.as_str()));
        let defaults = crate::providers::default_ids();
        for id in crate::providers::builtin_ids() {
            if !self.known_providers.contains(&id) {
                // a new source that needs an account is offered in Settings → Accounts, not switched on
                if defaults.contains(&id) && !self.providers.contains(&id) {
                    self.providers.push(id.clone());
                }
                self.known_providers.push(id);
            }
        }
    }

    /// Move an unreadable settings file aside (`config.toml.broken-<unix time>`) so the app can run
    /// on defaults without overwriting it. Returns where the file went, or `None` if there was none.
    pub fn quarantine_unreadable() -> Result<Option<PathBuf>> {
        let Some(path) = Self::path() else { return Ok(None) };
        Self::quarantine_file(&path)
    }

    fn quarantine_file(path: &Path) -> Result<Option<PathBuf>> {
        if !path.exists() {
            return Ok(None);
        }
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".broken-{stamp}"));
        let dest = path.with_file_name(name);
        std::fs::rename(path, &dest).with_context(|| format!("moving {} aside", path.display()))?;
        Ok(Some(dest))
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path().context("no config directory on this platform")?;
        self.save_to(&path)?;
        Ok(path)
    }

    /// Write atomically: a temporary file next to `path`, then a rename, so a crash or a full
    /// disk never leaves a half-written settings file behind.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        crate::atomic::write(path, toml::to_string_pretty(self)?.as_bytes())
    }

    /// Reject values that would only fail later (a bad title-clean regex, zero concurrency, a bad proxy).
    pub fn validate(&self) -> Result<()> {
        if let Some(rx) = self.tag_clean_pattern.as_deref().filter(|s| !s.trim().is_empty()) {
            regex::Regex::new(rx).with_context(|| format!("tag_clean_pattern is not a valid regex: `{rx}`"))?;
        }
        if !(0.0..=1.0).contains(&self.min_score) {
            anyhow::bail!("min_score must be between 0 and 1 (got {})", self.min_score);
        }
        if let Some(p) = self.proxy.as_deref() {
            reqwest::Proxy::all(p).with_context(|| format!("invalid proxy url `{p}`"))?;
        }
        Ok(())
    }

    /// Read one key as text, for `lyrixearch config get KEY`. Lists are comma-separated; unset options are empty.
    pub fn get(&self, key: &str) -> Result<String> {
        let list = |v: &[String]| v.join(",");
        let opt = |v: &Option<String>| v.clone().unwrap_or_default();
        Ok(match key {
            "providers" => list(&self.providers),
            "concurrency" => self.concurrency.to_string(),
            "provider_timeout_secs" => self.provider_timeout_secs.to_string(),
            "penalty_max_failures" => self.penalty_max_failures.to_string(),
            "penalty_window_secs" => self.penalty_window_secs.to_string(),
            "penalty_cooldown_secs" => self.penalty_cooldown_secs.to_string(),
            "write_tags" => self.write_tags.to_string(),
            "sidecar" => format!("{:?}", self.sidecar).to_ascii_lowercase(),
            "keep_mtime" => self.keep_mtime.to_string(),
            "word_timing" => self.word_timing.to_string(),
            "auto_fetch" => self.auto_fetch.to_string(),
            "keep_running" => self.keep_running.to_string(),
            "notify_done" => self.notify_done.to_string(),
            "keep_awake" => self.keep_awake.to_string(),
            "drop_shelf" => self.drop_shelf.to_string(),
            "overwrite" => self.overwrite.to_string(),
            "remove_section_tags" => self.remove_section_tags.to_string(),
            "prepend_newline" => self.prepend_newline.to_string(),
            "proxy" => opt(&self.proxy),
            "min_score" => self.min_score.to_string(),
            "search_engine" => self.search_engine.clone(),
            "library_dirs" => list(&self.library_dirs),
            "browse_roots" => list(&self.browse_roots),
            "tui_search_new" => self.tui_search_new.to_string(),
            "synced_only" => self.synced_only.to_string(),
            "lastfm_api_key" => opt(&self.lastfm_api_key),
            "tag_clean_enabled" => self.tag_clean_enabled.to_string(),
            "tag_clean_pattern" => opt(&self.tag_clean_pattern),
            _ => anyhow::bail!("unknown key `{key}`"),
        })
    }

    /// Set one key from a string, for `lyrixearch config set KEY VALUE`.
    pub fn set(&mut self, key: &str, value: &str) -> Result<()> {
        let b = |v: &str| v.parse::<bool>().map_err(|_| anyhow::anyhow!("expected true/false"));
        match key {
            "providers" => self.providers = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "concurrency" => self.concurrency = value.parse::<usize>()?.max(1),
            "provider_timeout_secs" => self.provider_timeout_secs = value.parse()?,
            "penalty_max_failures" => self.penalty_max_failures = value.parse()?,
            "penalty_window_secs" => self.penalty_window_secs = value.parse()?,
            "penalty_cooldown_secs" => self.penalty_cooldown_secs = value.parse()?,
            "write_tags" => self.write_tags = b(value)?,
            "sidecar" => self.sidecar = value.parse().map_err(|e: String| anyhow::anyhow!(e))?,
            "keep_mtime" => self.keep_mtime = b(value)?,
            "word_timing" => self.word_timing = b(value)?,
            "auto_fetch" => self.auto_fetch = b(value)?,
            "keep_running" => self.keep_running = b(value)?,
            "notify_done" => self.notify_done = b(value)?,
            "keep_awake" => self.keep_awake = b(value)?,
            "drop_shelf" => self.drop_shelf = b(value)?,
            "overwrite" => self.overwrite = b(value)?,
            "remove_section_tags" => self.remove_section_tags = b(value)?,
            "prepend_newline" => self.prepend_newline = b(value)?,
            "proxy" => self.proxy = if value.is_empty() { None } else { Some(value.to_string()) },
            "min_score" => self.min_score = value.parse()?,
            "search_engine" => self.search_engine = value.to_string(),
            "browse_roots" => self.browse_roots = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "tui_search_new" => self.tui_search_new = b(value)?,
            "synced_only" => self.synced_only = b(value)?,
            "lastfm_api_key" => self.lastfm_api_key = Some(value.trim().to_string()).filter(|s| !s.is_empty()),
            "tag_clean_enabled" => self.tag_clean_enabled = b(value)?,
            "tag_clean_pattern" => self.tag_clean_pattern = Some(value.to_string()).filter(|s| !s.trim().is_empty()),
            _ => anyhow::bail!("unknown key `{key}`"),
        }
        self.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_builtin_sources_reach_an_older_settings_file_once() {
        // written before AZLyrics and Letras existed, listing sources since removed; genius turned off
        let old = "providers = [\"lrclib\", \"netease\", \"lyricsfreak\", \"utaten\"]\n";
        let dir = std::env::temp_dir().join(format!("lyrixearch-cfg-migrate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, old).unwrap();
        let mut c = Config::load_from(&path).unwrap();
        assert_eq!(c.providers, ["lrclib", "lyricsfreak", "metal_archives", "azlyrics", "letras"], "removed sources go, new ones are added");
        assert!(!c.providers.contains(&"genius".to_string()), "a source the person turned off stays off");
        // once offered, turning a new one off sticks
        c.providers.retain(|p| p != "azlyrics");
        std::fs::write(&path, toml::to_string(&c).unwrap()).unwrap();
        let again = Config::load_from(&path).unwrap();
        assert!(!again.providers.contains(&"azlyrics".to_string()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lyrixearch-cfg-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn set_get_round_trip_for_every_key() {
        let mut c = Config::default();
        for (k, v) in [
            ("providers", "lrclib,genius"),
            ("concurrency", "4"),
            ("sidecar", "lrc"),
            ("keep_mtime", "false"),
            ("proxy", "http://127.0.0.1:8080"),
            ("synced_only", "true"),
            ("lastfm_api_key", "abc123"),
            ("tag_clean_enabled", "false"),
            ("tag_clean_pattern", r"\s*\(demo\)$"),
            ("browse_roots", "/a, /b"),
        ] {
            c.set(k, v).unwrap_or_else(|e| panic!("set {k}: {e:#}"));
        }
        assert_eq!(c.get("providers").unwrap(), "lrclib,genius");
        assert_eq!(c.get("concurrency").unwrap(), "4");
        assert_eq!(c.get("sidecar").unwrap(), "lrc");
        assert_eq!(c.get("keep_mtime").unwrap(), "false");
        assert_eq!(c.get("synced_only").unwrap(), "true");
        assert_eq!(c.get("lastfm_api_key").unwrap(), "abc123");
        assert_eq!(c.get("tag_clean_pattern").unwrap(), r"\s*\(demo\)$");
        assert_eq!(c.get("browse_roots").unwrap(), "/a,/b");
        // empty value clears an option
        c.set("lastfm_api_key", "").unwrap();
        assert_eq!(c.lastfm_api_key, None);
        assert!(c.get("nope").is_err());
        assert!(c.set("nope", "1").is_err());
    }

    #[test]
    fn set_rejects_bad_values() {
        let mut c = Config::default();
        assert!(c.set("tag_clean_pattern", "(unclosed").is_err());
        assert!(c.set("keep_mtime", "maybe").is_err());
        assert!(c.set("min_score", "3").is_err());
        assert!(c.set("sidecar", "zip").is_err());
    }

    #[test]
    fn save_is_atomic_and_loads_back() {
        let dir = temp_dir("save");
        let path = dir.join("config.toml");
        let c = Config { concurrency: 3, synced_only: true, ..Config::default() };
        c.save_to(&path).unwrap();
        let back = Config::load_from(&path).unwrap();
        assert_eq!(back.concurrency, 3);
        assert!(back.synced_only);
        // no temporary files left beside it
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from("config.toml")]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_files_without_new_keys_still_load() {
        let dir = temp_dir("old");
        let path = dir.join("config.toml");
        std::fs::write(&path, "concurrency = 2\nsidecar = \"off\"\n").unwrap();
        let c = Config::load_from(&path).unwrap();
        assert_eq!(c.concurrency, 2);
        assert!(c.tag_clean_enabled, "new keys take their defaults");
        assert!(!c.synced_only);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_file_is_an_error_and_can_be_moved_aside() {
        let dir = temp_dir("broken");
        let path = dir.join("config.toml");
        std::fs::write(&path, "concurrency = \"not a number\"\n").unwrap();
        assert!(Config::load_from(&path).is_err());
        let moved = Config::quarantine_file(&path).unwrap().unwrap();
        assert!(!path.exists());
        assert!(moved.file_name().unwrap().to_string_lossy().starts_with("config.toml.broken-"));
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "concurrency = \"not a number\"\n");
        assert_eq!(Config::quarantine_file(&path).unwrap(), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
