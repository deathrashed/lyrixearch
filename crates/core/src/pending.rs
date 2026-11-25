//! Found lyrics that have not been written yet, kept between runs (the original's
//! `pendingFoundLyrics`). Stored as JSON in `<config>/lyrixearch/pending.json`, keyed by the
//! audio file's path, so quitting before pressing "embed" does not lose a search.

use crate::config::Config;
use crate::model::Lyrics;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Unsaved results by audio path.
pub type Pending = HashMap<PathBuf, Lyrics>;

/// `<config>/lyrixearch/pending.json`.
pub fn path() -> Option<PathBuf> {
    Config::config_dir().map(|d| d.join("pending.json"))
}

/// Load the saved results; a missing file is an empty set. Entries whose audio file no longer
/// exists are dropped.
pub fn load() -> Result<Pending> {
    match path() {
        Some(p) => load_from(&p),
        None => Ok(Pending::new()),
    }
}

/// Replace the saved set with `pending` (an empty set removes the file).
pub fn save(pending: &Pending) -> Result<()> {
    let p = path().context("no config directory on this platform")?;
    save_to(&p, pending)
}

pub fn load_from(p: &Path) -> Result<Pending> {
    let text = match std::fs::read_to_string(p) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Pending::new()),
        Err(e) => return Err(e).with_context(|| format!("reading {}", p.display())),
    };
    let mut map: Pending = serde_json::from_str(&text).with_context(|| format!("parsing {}", p.display()))?;
    map.retain(|audio, _| audio.exists());
    Ok(map)
}

pub fn save_to(p: &Path, pending: &Pending) -> Result<()> {
    if pending.is_empty() {
        return match std::fs::remove_file(p) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e).with_context(|| format!("removing {}", p.display())),
            _ => Ok(()),
        };
    }
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    crate::atomic::write(p, serde_json::to_string_pretty(pending)?.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_drops_missing_files_and_empty_set_removes_the_file() {
        let dir = std::env::temp_dir().join(format!("lyrixearch-pending-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("pending.json");
        let audio = dir.join("song.mp3");
        std::fs::write(&audio, b"x").unwrap();

        let mut map = Pending::new();
        map.insert(audio.clone(), Lyrics { plain: Some("words".into()), source: "lrclib".into(), ..Default::default() });
        map.insert(dir.join("gone.mp3"), Lyrics::default());
        save_to(&store, &map).unwrap();

        let back = load_from(&store).unwrap();
        assert_eq!(back.len(), 1, "the entry for a file that no longer exists is dropped");
        assert_eq!(back[&audio].plain.as_deref(), Some("words"));

        save_to(&store, &Pending::new()).unwrap();
        assert!(!store.exists());
        assert!(load_from(&store).unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
