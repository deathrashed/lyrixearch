//! Lyrics sources. Each implements [`Provider`]; add a new source by adding a file here and
//! one line in [`all`]. Providers do their own candidate selection with [`crate::matching`].

use crate::config::Config;
use crate::model::{Lyrics, Query};
use async_trait::async_trait;
use std::sync::Arc;

mod bandcamp;
mod darklyrics;
mod ddg;
mod genius;
mod kugou;
mod local;
pub use local::{find_sidecar, find_sidecar_kind};
mod lrclib;
mod lyricsfreak;
mod netease;
mod ovh;
pub mod plugin;
mod qq;
mod utaten;
pub mod web;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable id used in config and output (`lrclib`, `netease`, ...).
    fn id(&self) -> &str;

    /// `Ok(None)` means "searched fine, nothing suitable". `Err` means the provider itself
    /// misbehaved (network, bad JSON, blocked) and counts towards its penalty.
    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> anyhow::Result<Option<Lyrics>>;

    /// How long one lookup may take, when this source needs longer (or less) than the engine's default.
    fn timeout(&self) -> Option<std::time::Duration> {
        None
    }

    /// `(requests in flight, minimum gap between request starts)` for sources that know better than [`politeness`].
    fn politeness(&self) -> Option<(usize, std::time::Duration)> {
        None
    }

    /// Whether running out of time counts as a failure towards the penalty box. A slow source of your own
    /// (a plugin) should not be benched for being slow.
    fn timeouts_count(&self) -> bool {
        true
    }

    /// True if this provider can read lyrics straight from `url` (used by the web-search fallback).
    fn supports_url(&self, _url: &str) -> bool {
        false
    }

    /// Read lyrics from a page this provider recognises; `q` is used to verify it is the right song.
    async fn fetch_url(&self, _http: &reqwest::Client, _url: &str, _q: &Query, _cfg: &Config) -> anyhow::Result<Option<Lyrics>> {
        Ok(None)
    }
}

/// Built-in sources, in the original's default order, plus every plugin found in `plugins_dir`.
pub fn all(plugins_dir: &std::path::Path) -> Vec<Arc<dyn Provider>> {
    let genius: Arc<dyn Provider> = Arc::new(genius::Genius);
    let lyricsfreak: Arc<dyn Provider> = Arc::new(lyricsfreak::LyricsFreak);
    let ddg: Arc<dyn Provider> = Arc::new(ddg::DuckDuckGo { scrapers: vec![genius.clone(), lyricsfreak.clone()] });
    let mut v: Vec<Arc<dyn Provider>> = vec![
        Arc::new(local::LocalFiles),
        genius,
        Arc::new(lrclib::Lrclib),
        lyricsfreak,
        Arc::new(darklyrics::DarkLyrics),
        ddg,
        Arc::new(netease::Netease),
        Arc::new(qq::QqMusic),
        Arc::new(kugou::Kugou),
        Arc::new(utaten::UtaTen),
        Arc::new(bandcamp::Bandcamp),
        Arc::new(ovh::LyricsOvh),
    ];
    let (plugins, _broken) = plugin::discover(plugins_dir);
    for p in plugins {
        // A plugin may not shadow a built-in id.
        if !v.iter().any(|b| b.id() == p.id()) {
            v.push(Arc::new(p));
        }
    }
    v
}

/// Display info for settings screens.
#[derive(Clone, Debug, serde::Serialize)]
pub struct ProviderInfo {
    pub id: String,
    pub name: String,
    /// `builtin` or `plugin`.
    pub kind: &'static str,
    pub note: String,
}

/// Every source available (built-ins plus plugins), with human-readable names.
pub fn catalog(plugins_dir: &std::path::Path) -> Vec<ProviderInfo> {
    let builtin = |id: &str, name: &str, note: &str| ProviderInfo { id: id.into(), name: name.into(), kind: "builtin", note: note.into() };
    let mut v = vec![
        builtin("local_files", "Local files", ".lrc / .txt next to the track, or in a Lyrics folder"),
        builtin("genius", "Genius", "Plain lyrics, wide catalogue"),
        builtin("lrclib", "LRCLIB", "Open database with synced lyrics"),
        builtin("lyricsfreak", "LyricsFreak", "Plain lyrics"),
        builtin("darklyrics", "DarkLyrics", "Metal lyrics, album pages"),
        builtin("ddg_search", "Web search (DuckDuckGo)", "Finds a supported page when the others miss"),
        builtin("netease", "NetEase Cloud Music", "Synced lyrics, strong for Asian music"),
        builtin("qqmusic", "QQ Music", "Synced lyrics"),
        builtin("kugou", "Kugou", "Synced lyrics"),
        builtin("utaten", "UtaTen", "Japanese lyrics (only asked for Japanese titles)"),
        builtin("bandcamp", "Bandcamp", "Lyrics published by the artist on the track page"),
        builtin("lyrics_ovh", "lyrics.ovh", "Plain lyrics, exact names only"),
    ];
    let (plugins, _) = plugin::discover(plugins_dir);
    for p in plugins {
        let m = p.manifest();
        v.push(ProviderInfo {
            id: m.id.clone(),
            name: if m.name.is_empty() { m.id.clone() } else { m.name.clone() },
            kind: "plugin",
            note: p.dir().display().to_string(),
        });
    }
    v
}

/// A source answered with a bot check (captcha, browser challenge). A person, or a real browser
/// window, can pass it; the app opens one and shares the resulting cookies with the engine.
#[derive(Debug, Clone)]
pub struct Challenge {
    pub url: String,
    pub reason: String,
}

impl std::fmt::Display for Challenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.reason)
    }
}

impl std::error::Error for Challenge {}

/// A source said "slow down" (HTTP 429, or 503 with `Retry-After`). The engine leaves that source alone
/// for the time it asked for instead of counting failures until a penalty kicks in.
#[derive(Debug, Clone)]
pub struct RateLimited {
    pub retry_after: std::time::Duration,
    pub reason: String,
}

impl std::fmt::Display for RateLimited {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (asked us to wait {}s)", self.reason, self.retry_after.as_secs())
    }
}

impl std::error::Error for RateLimited {}

impl RateLimited {
    /// Build from a response: `Retry-After` seconds when present, otherwise a default wait.
    pub fn from_response(resp: &reqwest::Response, reason: &str) -> Self {
        let secs = resp.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok()).unwrap_or(30);
        // a server that says "1 second" while failing every request still needs a real pause
        Self { retry_after: std::time::Duration::from_secs(secs.clamp(15, 900)), reason: reason.into() }
    }
}

/// How hard one source may be hit, as `(requests in flight, minimum gap between request starts)`.
/// Searching many tracks at once multiplies requests per source, and small sites answer that with
/// captchas and 429s, so the polite sources get one or two at a time with a pause between.
pub fn politeness(id: &str) -> (usize, std::time::Duration) {
    let ms = std::time::Duration::from_millis;
    match id {
        "local_files" => (64, ms(0)),
        "ddg_search" => (1, ms(1000)),
        "genius" => (2, ms(500)),
        "darklyrics" => (1, ms(800)),
        "lyricsfreak" | "utaten" | "bandcamp" | "lyrics_ovh" => (2, ms(400)),
        "lrclib" => (4, ms(100)),
        "netease" | "qqmusic" | "kugou" => (3, ms(200)),
        _ => (2, ms(300)),
    }
}

pub const USER_AGENT: &str = concat!("lyrixearch/", env!("CARGO_PKG_VERSION"));
