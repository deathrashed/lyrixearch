use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Same order the original ships with, restricted to sources this build implements.
            providers: [
                "local_files",
                "genius",
                "lrclib",
                "lyricsfreak",
                "darklyrics",
                "ddg_search",
                "netease",
                "qqmusic",
                "kugou",
                "utaten",
                "bandcamp",
                "lyrics_ovh",
            ]
            .map(String::from)
            .to_vec(),
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
                if let Some(parent) = new.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::rename(&old, &new);
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

    /// Load from the default location; missing file means defaults.
    pub fn load() -> Result<Self> {
        let Some(path) = Self::path() else { return Ok(Self::default()) };
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s).with_context(|| format!("parsing {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::path().context("no config directory on this platform")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, toml::to_string_pretty(self)?)?;
        Ok(path)
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
            "overwrite" => self.overwrite = b(value)?,
            "remove_section_tags" => self.remove_section_tags = b(value)?,
            "prepend_newline" => self.prepend_newline = b(value)?,
            "proxy" => self.proxy = if value.is_empty() { None } else { Some(value.to_string()) },
            "min_score" => self.min_score = value.parse()?,
            "search_engine" => self.search_engine = value.to_string(),
            "browse_roots" => self.browse_roots = value.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            "tui_search_new" => self.tui_search_new = b(value)?,
            _ => anyhow::bail!("unknown key `{key}`"),
        }
        Ok(())
    }
}
