//! Lyrics sources. Each implements [`Provider`]. To add a built-in source:
//! 1. add a file here implementing [`Provider`],
//! 2. construct it in [`all`],
//! 3. add its row to [`BUILTINS`] (name, note, politeness; the row order is the default search order).
//!
//! A test checks that [`all`] and [`BUILTINS`] list the same ids in the same order.
//! Providers do their own candidate selection with [`crate::matching`].

use crate::config::Config;
use crate::model::{Lyrics, Query};
use async_trait::async_trait;
use std::sync::Arc;

pub mod accounts;
mod azlyrics;
mod bandcamp;
mod darklyrics;
mod ddg;
mod genius;
mod letras;
mod local;
pub use local::{find_sidecar, find_sidecar_kind};
mod lrclib;
mod lyricsfreak;
mod metal_archives;
mod ovh;
pub mod plugin;
pub mod transcribe;
pub mod web;

#[async_trait]
pub trait Provider: Send + Sync {
    /// Stable id used in config and output (`lrclib`, `genius`, ...).
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
    let darklyrics: Arc<dyn Provider> = Arc::new(darklyrics::DarkLyrics);
    let bandcamp: Arc<dyn Provider> = Arc::new(bandcamp::Bandcamp);
    let azlyrics: Arc<dyn Provider> = Arc::new(azlyrics::AzLyrics);
    let letras: Arc<dyn Provider> = Arc::new(letras::Letras);
    // Letras' song pages refuse plain HTTP clients outright (Akamai), so they always load in the browser
    web::route_through_browser(letras::BASE);
    web::route_through_browser("https://www.letras.mus.br/");
    // every scraper that can read a page straight from a URL; the web search hands results to them
    let scrapers = vec![genius.clone(), azlyrics.clone(), letras.clone(), lyricsfreak.clone(), darklyrics.clone(), bandcamp.clone()];
    let ddg: Arc<dyn Provider> = Arc::new(ddg::DuckDuckGo { scrapers });
    let mut v: Vec<Arc<dyn Provider>> = vec![
        Arc::new(local::LocalFiles),
        Arc::new(lrclib::Lrclib),
        genius,
        lyricsfreak,
        darklyrics,
        bandcamp,
        Arc::new(ovh::LyricsOvh),
        Arc::new(metal_archives::MetalArchives),
        azlyrics,
        letras,
        ddg,
    ];
    // the account sources: only used once switched on (Settings → Accounts signs in and enables them)
    for (id, name) in accounts::SERVICES {
        v.push(Arc::new(accounts::AccountSource { id, name }));
    }
    v.push(Arc::new(transcribe::Transcribe));
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

/// A built-in source's display details and how hard it may be hit.
pub struct Builtin {
    pub id: &'static str,
    pub name: &'static str,
    pub note: &'static str,
    /// Requests in flight at once.
    pub inflight: usize,
    /// Minimum gap between request starts, in milliseconds.
    pub gap_ms: u64,
    /// In the default source list. The account sources are not: they need signing in first.
    pub default_on: bool,
}

const fn builtin(id: &'static str, name: &'static str, note: &'static str, inflight: usize, gap_ms: u64) -> Builtin {
    Builtin { id, name, note, inflight, gap_ms, default_on: true }
}

const fn account(id: &'static str, name: &'static str, note: &'static str) -> Builtin {
    Builtin { id, name, note, inflight: 1, gap_ms: 300, default_on: false }
}

/// Every built-in source, in the default search order (the original's order).
///
/// Politeness: searching many tracks at once multiplies requests per source, and small sites answer
/// that with captchas and 429s, so the polite sources get one or two at a time with a pause between.
pub const BUILTINS: &[Builtin] = &[
    // fast APIs first; sites that load in the app's browser (several seconds a page) after them;
    // the web search last, as it only helps when the others miss
    builtin("local_files", "Local files", ".lrc / .txt next to the track, or in a Lyrics folder", 64, 0),
    builtin("lrclib", "LRCLIB", "Open database with synced lyrics", 4, 100),
    builtin("genius", "Genius", "Plain lyrics, wide catalogue", 2, 500),
    builtin("lyricsfreak", "LyricsFreak", "Plain lyrics", 2, 400),
    builtin("darklyrics", "DarkLyrics", "Metal lyrics, album pages", 1, 800),
    builtin("bandcamp", "Bandcamp", "Lyrics published by the artist on the track page", 2, 400),
    builtin("lyrics_ovh", "lyrics.ovh", "Plain lyrics, exact names only", 2, 400),
    // a small community site: one request at a time, with a pause
    builtin("metal_archives", "Metal Archives", "Metal lyrics, song by song", 1, 1200),
    builtin("azlyrics", "AZLyrics", "Plain lyrics, large English catalogue", 1, 1500),
    builtin("letras", "Letras", "Large catalogue, strong for Portuguese and Spanish music", 1, 800),
    builtin("ddg_search", "Web search (DuckDuckGo)", "Finds a supported page when the others miss", 1, 2500),
    account("spotify", "Spotify", "Synced lyrics, with your Spotify account (Settings → Accounts)"),
    account("apple_music", "Apple Music", "Synced lyrics, with an Apple Music subscription (Settings → Accounts)"),
    account("deezer", "Deezer", "Synced lyrics, with your Deezer account (Settings → Accounts)"),
    // writes the lyrics down from the track itself; last, and only when switched on
    Builtin {
        id: "ai_transcribe",
        name: "AI transcription",
        note: "Whisper writes the lyrics down from the track; check before embedding (Settings → Transcription)",
        inflight: 1,
        gap_ms: 0,
        default_on: false,
    },
];

/// The built-in sources before AZLyrics and Letras were added. A settings file written then
/// lists its sources explicitly, so sources added since are switched on when it is loaded; ones the
/// person had already seen and turned off stay off (see `Config::known_providers`).
pub const ORIGINAL_BUILTINS: &[&str] =
    &["local_files", "genius", "lrclib", "lyricsfreak", "darklyrics", "ddg_search", "netease", "qqmusic", "kugou", "utaten", "bandcamp", "lyrics_ovh"];

/// Sources that were built in and have been taken out (the Chinese and Japanese sources). They are
/// dropped from a settings file's source list when it is loaded.
pub const REMOVED_BUILTINS: &[&str] = &["netease", "qqmusic", "kugou", "utaten", "utanet"];

/// Ids of the built-in sources in default search order.
pub fn builtin_ids() -> Vec<String> {
    BUILTINS.iter().map(|b| b.id.to_string()).collect()
}

/// The sources a new settings file starts with (every built-in except the account ones).
pub fn default_ids() -> Vec<String> {
    BUILTINS.iter().filter(|b| b.default_on).map(|b| b.id.to_string()).collect()
}

/// Every source available (built-ins plus plugins), with human-readable names.
pub fn catalog(plugins_dir: &std::path::Path) -> Vec<ProviderInfo> {
    let mut v: Vec<ProviderInfo> =
        BUILTINS.iter().map(|b| ProviderInfo { id: b.id.into(), name: b.name.into(), kind: "builtin", note: b.note.into() }).collect();
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
/// Built-ins take their row in [`BUILTINS`]; anything else (plugins) gets a cautious default.
pub fn politeness(id: &str) -> (usize, std::time::Duration) {
    BUILTINS.iter().find(|b| b.id == id).map_or((2, std::time::Duration::from_millis(300)), |b| (b.inflight, std::time::Duration::from_millis(b.gap_ms)))
}

/// Status handling every HTTP source shares: "slow down" answers (429, or 503) become
/// [`RateLimited`] so the engine waits as asked; other failures become errors.
pub trait CheckStatus: Sized {
    fn check_status(self, who: &str) -> anyhow::Result<Self>;
}

impl CheckStatus for reqwest::Response {
    fn check_status(self, who: &str) -> anyhow::Result<Self> {
        match self.status().as_u16() {
            429 => Err(RateLimited::from_response(&self, &format!("{who} rate limited (HTTP 429)")).into()),
            503 => Err(RateLimited::from_response(&self, &format!("{who} busy (HTTP 503)")).into()),
            _ => self.error_for_status().map_err(|e| anyhow::anyhow!("{who}: {e}")),
        }
    }
}

pub const USER_AGENT: &str = concat!("lyrixearch/", env!("CARGO_PKG_VERSION"));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_table_matches_the_constructed_sources() {
        let empty = std::env::temp_dir().join(format!("lyrixearch-no-plugins-{}", std::process::id()));
        let ids: Vec<String> = all(&empty).iter().map(|p| p.id().to_string()).collect();
        assert_eq!(ids, builtin_ids(), "all() and BUILTINS must list the same sources in the same order");
        assert_eq!(crate::config::Config::default().providers, default_ids(), "a new setup starts without the account sources");
    }

    #[test]
    fn politeness_falls_back_for_unknown_ids() {
        assert_eq!(politeness("darklyrics"), (1, std::time::Duration::from_millis(800)));
        assert_eq!(politeness("some_plugin"), (2, std::time::Duration::from_millis(300)));
    }

    #[test]
    fn retry_after_is_floored_and_capped() {
        let resp = |h: Option<&str>| {
            let mut b = http::Response::builder().status(429);
            if let Some(v) = h {
                b = b.header("Retry-After", v);
            }
            reqwest::Response::from(b.body("").unwrap())
        };
        assert_eq!(RateLimited::from_response(&resp(Some("1")), "x").retry_after.as_secs(), 15);
        assert_eq!(RateLimited::from_response(&resp(Some("120")), "x").retry_after.as_secs(), 120);
        assert_eq!(RateLimited::from_response(&resp(Some("99999")), "x").retry_after.as_secs(), 900);
        assert_eq!(RateLimited::from_response(&resp(None), "x").retry_after.as_secs(), 30);
        let err = resp(Some("20")).check_status("src").unwrap_err();
        assert_eq!(err.downcast_ref::<RateLimited>().unwrap().retry_after.as_secs(), 20);
    }
}
