//! Lyrics files that already sit next to the track: `Song.lrc`, `Lyrics/Song.lrc`, `Song.txt`.
//! (Handy for embedding a library you have already collected lyrics for.)

use super::Provider;
use crate::config::Config;
use crate::lrc;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct LocalFiles;

fn candidates(audio: &Path, only: Option<&str>) -> Vec<PathBuf> {
    let stem = audio.file_stem().map(|s| s.to_os_string()).unwrap_or_default();
    let dir = audio.parent().unwrap_or(Path::new("."));
    let mut out = Vec::new();
    for ext in ["lrc", "txt"].into_iter().filter(|e| only.is_none_or(|o| o.eq_ignore_ascii_case(e))) {
        let name = {
            let mut n = stem.clone();
            n.push(format!(".{ext}"));
            n
        };
        out.push(dir.join(&name));
        out.push(dir.join("Lyrics").join(&name));
        out.push(dir.join("lyrics").join(&name));
    }
    out
}

/// First `.lrc` / `.txt` that belongs to this track (same folder or a `Lyrics/` subfolder), with its text.
pub fn find_sidecar(audio: &Path) -> Option<(PathBuf, String)> {
    find_sidecar_kind(audio, None)
}

/// Like [`find_sidecar`], limited to `lrc` or `txt` when `only` names one (anything else means both).
pub fn find_sidecar_kind(audio: &Path, only: Option<&str>) -> Option<(PathBuf, String)> {
    let only = only.filter(|o| matches!(*o, "lrc" | "txt"));
    candidates(audio, only).into_iter().find_map(|c| read_text(&c).map(|t| (c, t)))
}

fn read_text(p: &Path) -> Option<String> {
    let bytes = std::fs::read(p).ok()?;
    let s = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').trim().to_string();
    (!s.is_empty()).then_some(s)
}

#[async_trait]
impl Provider for LocalFiles {
    fn id(&self) -> &str {
        "local_files"
    }

    async fn fetch(&self, _http: &reqwest::Client, q: &Query, _cfg: &Config) -> Result<Option<Lyrics>> {
        let Some(path) = q.source_path.as_deref().map(Path::new) else { return Ok(None) };
        for c in candidates(path, None) {
            let Some(text) = read_text(&c) else { continue };
            let (plain, synced) = if lrc::looks_synced(&text) { (Some(lrc::strip_timestamps(&text)), Some(text)) } else { (Some(text), None) };
            return Ok(Some(Lyrics {
                plain,
                synced,
                source: "local_files".into(),
                url: Some(c.to_string_lossy().into_owned()),
                matched_artist: q.artist.clone(),
                matched_title: q.title.clone(),
                score: 1.0,
            }));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_kind_limits_the_search() {
        let dir = std::env::temp_dir().join(format!("lf-kind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let audio = dir.join("b.mp3");
        std::fs::write(dir.join("b.lrc"), "[00:01.00]synced\n").unwrap();
        std::fs::write(dir.join("b.txt"), "plain\n").unwrap();
        assert!(find_sidecar_kind(&audio, Some("txt")).unwrap().0.ends_with("b.txt"));
        assert!(find_sidecar_kind(&audio, Some("lrc")).unwrap().0.ends_with("b.lrc"));
        assert!(find_sidecar_kind(&audio, None).unwrap().0.ends_with("b.lrc"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_sidecar_next_to_track() {
        let dir = std::env::temp_dir().join(format!("lf-local-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("Lyrics")).unwrap();
        let audio = dir.join("a.mp3");
        std::fs::write(dir.join("Lyrics").join("a.lrc"), "[00:01.00]hello\n").unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let q = Query { source_path: Some(audio.to_string_lossy().into()), ..Default::default() };
        let got = rt.block_on(LocalFiles.fetch(&reqwest::Client::new(), &q, &Config::default())).unwrap().unwrap();
        assert!(got.synced.is_some());
        assert_eq!(got.plain.as_deref(), Some("hello"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
