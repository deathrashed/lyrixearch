use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What we ask providers for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Query {
    pub artist: String,
    pub title: String,
    pub album: Option<String>,
    /// Retried in place of `artist` when that is a "Various Artists" placeholder.
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub duration_secs: Option<u32>,
    /// Path of the audio file this query came from (lets local and plugin sources look around it).
    pub source_path: Option<String>,
}

/// A lyrics result, normalised across providers.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Lyrics {
    /// Plain (unsynchronised) text.
    pub plain: Option<String>,
    /// LRC text with `[mm:ss.xx]` timestamps.
    pub synced: Option<String>,
    pub source: String,
    pub url: Option<String>,
    pub matched_artist: String,
    pub matched_title: String,
    /// 0.0..=1.0 match confidence against the query.
    pub score: f32,
    /// Found but to be checked by a person before it goes into the file (AI transcriptions): a
    /// search shows it, Fetch does not write it by itself.
    #[serde(default)]
    pub review: bool,
}

impl Lyrics {
    /// Text to embed in tags: plain if present, otherwise the synced text with timestamps removed.
    /// Credit lines (lyricist, composer) are dropped, and `None` means there is no real lyric.
    pub fn embed_text(&self) -> Option<String> {
        let raw = match self.plain.as_deref().filter(|s| !s.trim().is_empty()) {
            Some(p) => p.to_string(),
            None => self.synced.as_deref().map(crate::lrc::strip_timestamps)?,
        };
        let text = crate::lrc::strip_credits(&raw);
        crate::lrc::is_substantial(&text).then_some(text)
    }

    pub fn has_text(&self) -> bool {
        self.embed_text().is_some()
    }
}

/// One audio file in the working list.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Track {
    pub path: PathBuf,
    pub query: Query,
    pub has_lyrics: bool,
    pub format: String,
}

impl Track {
    pub fn display_name(&self) -> String {
        self.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
    }
}
