//! Shared engine for the `lyrixearch` CLI/TUI and GUI.
//!
//! Flow: [`scan::collect_audio`] -> [`tags::read_track`] -> [`engine::Engine::run`]
//! -> [`engine::Engine::apply`] (tags and/or sidecar file).

pub mod atomic;
pub mod clean;
pub mod config;
pub mod engine;
pub mod lastfm;
pub mod lrc;
pub mod matching;
pub mod model;
pub mod output;
pub mod pending;
pub mod providers;
pub mod scan;
pub mod tags;
pub mod util;

pub use config::{Config, Sidecar};
pub use engine::{Applied, Engine, Event, Outcome, Step};
pub use model::{Lyrics, Query, Track};
pub use output::SidecarEmbed;

/// The HTTP client type providers use (re-exported so front ends need not depend on `reqwest`).
pub use reqwest::cookie::Jar as CookieJar;
pub use reqwest::Client as HttpClient;
