//! Turning a provider result into what lands on disk: text post-processing and sidecar files.

use crate::config::{Config, Sidecar};
use crate::lrc;
use crate::model::Lyrics;
use once_cell::sync::Lazy;
use regex::Regex;
use std::path::{Path, PathBuf};

static SECTION_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\[[^\[\]]+\]\s*$").unwrap());
static MANY_BLANKS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

/// Normalise line endings, collapse runs of blank lines, optionally drop `[Chorus]` lines
/// and prepend a blank line — mirrors the options of the original app.
pub fn postprocess(text: &str, cfg: &Config) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = if cfg.remove_section_tags { text.lines().filter(|l| !SECTION_LINE.is_match(l)).collect::<Vec<_>>().join("\n") } else { text };
    let text = MANY_BLANKS.replace_all(&text, "\n\n").trim().to_string();
    if cfg.prepend_newline {
        format!("\n{text}")
    } else {
        text
    }
}

/// Where a sidecar for `track` would go and what it would contain, per the configured mode.
pub fn sidecar_plan(track: &Path, lyrics: &Lyrics, mode: Sidecar, cfg: &Config) -> Option<(PathBuf, String)> {
    let synced = lyrics.synced.as_deref().filter(|s| lrc::looks_synced(s));
    let plain = lyrics.embed_text();
    let (ext, body) = match mode {
        Sidecar::Off => return None,
        Sidecar::Lrc => ("lrc", synced?.to_string()),
        Sidecar::Txt => ("txt", plain?),
        Sidecar::Auto => match synced {
            Some(s) => ("lrc", s.to_string()),
            None => ("txt", plain?),
        },
    };
    let body = if ext == "txt" { postprocess(&body, cfg) } else { body };
    Some((track.with_extension(ext), body))
}

pub fn write_sidecar(track: &Path, lyrics: &Lyrics, cfg: &Config) -> anyhow::Result<Option<PathBuf>> {
    match sidecar_plan(track, lyrics, cfg.sidecar, cfg) {
        Some((path, body)) => {
            std::fs::write(&path, body)?;
            Ok(Some(path))
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn postprocess_options() {
        let cfg = Config { remove_section_tags: true, ..Config::default() };
        let out = postprocess("[Verse 1]\nline a\r\n\r\n\r\n\r\nline b\n[Chorus]\nline c", &cfg);
        assert_eq!(out, "line a\n\nline b\nline c");
    }

    #[test]
    fn sidecar_auto_prefers_lrc() {
        let cfg = Config::default();
        let l =
            Lyrics { plain: Some("first line\nsecond line".into()), synced: Some("[00:01.00]first line\n[00:02.00]second line".into()), ..Default::default() };
        let (p, _) = sidecar_plan(Path::new("/m/x.mp3"), &l, Sidecar::Auto, &cfg).unwrap();
        assert_eq!(p, PathBuf::from("/m/x.lrc"));
        let l = Lyrics { plain: Some("first line\nsecond line".into()), ..Default::default() };
        let (p, _) = sidecar_plan(Path::new("/m/x.mp3"), &l, Sidecar::Auto, &cfg).unwrap();
        assert_eq!(p, PathBuf::from("/m/x.txt"));
    }
}
