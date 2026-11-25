//! Turning a provider result into what lands on disk: text post-processing and sidecar files.

use crate::config::{Config, Sidecar};
use crate::lrc;
use crate::model::{Lyrics, Track};
use crate::{providers, tags};
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

/// Write the sidecar the config asks for. An existing sidecar with different content is only
/// replaced when `cfg.overwrite` is on, so a hand-made `.lrc`/`.txt` is never lost to an
/// automatic search. Returns the sidecar path when it now holds this result.
pub fn write_sidecar(track: &Path, lyrics: &Lyrics, cfg: &Config) -> anyhow::Result<Option<PathBuf>> {
    let Some((path, body)) = sidecar_plan(track, lyrics, cfg.sidecar, cfg) else { return Ok(None) };
    if let Ok(existing) = std::fs::read_to_string(&path) {
        if existing == body {
            return Ok(Some(path));
        }
        if !cfg.overwrite {
            return Ok(None);
        }
    }
    crate::atomic::write(&path, body.as_bytes())?;
    Ok(Some(path))
}

/// What [`embed_from_sidecar`] did for one track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidecarEmbed {
    /// Lyrics from this `.lrc`/`.txt` were written into the track.
    Embedded(PathBuf),
    /// Dry run: this file would have been embedded.
    WouldEmbed(PathBuf),
    /// No matching `.lrc`/`.txt` (or it was empty).
    NoFile,
    /// The track already has lyrics and `overwrite` was off.
    AlreadyHad,
}

/// Embed the `.lrc`/`.txt` that belongs to `track` (same folder, or a `Lyrics/` subfolder).
/// `kind` limits the search to `"lrc"` or `"txt"`; anything else means both, `.lrc` first.
/// An `.lrc` also fills the synced-lyrics field where the format has one.
pub fn embed_from_sidecar(track: &Track, cfg: &Config, overwrite: bool, kind: Option<&str>, dry_run: bool) -> anyhow::Result<SidecarEmbed> {
    if track.has_lyrics && !overwrite {
        return Ok(SidecarEmbed::AlreadyHad);
    }
    let Some((file, raw)) = providers::find_sidecar_kind(&track.path, kind) else { return Ok(SidecarEmbed::NoFile) };
    let synced = lrc::looks_synced(&raw).then_some(raw.as_str());
    let plain = match synced {
        Some(s) => lrc::strip_timestamps(s),
        None => raw.clone(),
    };
    let text = postprocess(&plain, cfg);
    if text.trim().is_empty() {
        return Ok(SidecarEmbed::NoFile);
    }
    if dry_run {
        return Ok(SidecarEmbed::WouldEmbed(file));
    }
    tags::write_lyrics_with(&track.path, &text, synced, cfg.keep_mtime)?;
    Ok(SidecarEmbed::Embedded(file))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("lyrixearch-out-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

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

    #[test]
    fn write_sidecar_keeps_a_different_existing_file_unless_overwrite() {
        let dir = temp_dir("keep");
        let track = dir.join("song.mp3");
        let hand_made = dir.join("song.txt");
        std::fs::write(&hand_made, "my own careful transcription").unwrap();
        let l = Lyrics { plain: Some("found line one\nfound line two".into()), ..Default::default() };

        let cfg = Config { sidecar: Sidecar::Txt, overwrite: false, ..Config::default() };
        assert_eq!(write_sidecar(&track, &l, &cfg).unwrap(), None);
        assert_eq!(std::fs::read_to_string(&hand_made).unwrap(), "my own careful transcription");

        let cfg = Config { sidecar: Sidecar::Txt, overwrite: true, ..Config::default() };
        assert_eq!(write_sidecar(&track, &l, &cfg).unwrap(), Some(hand_made.clone()));
        assert_eq!(std::fs::read_to_string(&hand_made).unwrap(), "found line one\nfound line two");

        // same content again: reported as written, nothing to replace
        let cfg = Config { sidecar: Sidecar::Txt, overwrite: false, ..Config::default() };
        assert_eq!(write_sidecar(&track, &l, &cfg).unwrap(), Some(hand_made));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn embed_from_sidecar_reports_each_case() {
        let dir = temp_dir("embed");
        let track = Track { path: dir.join("missing.mp3"), query: Default::default(), has_lyrics: false, format: "mp3".into() };
        let cfg = Config::default();
        assert_eq!(embed_from_sidecar(&track, &cfg, false, None, false).unwrap(), SidecarEmbed::NoFile);

        std::fs::write(dir.join("missing.lrc"), "[00:01.00]a line\n[00:02.00]another").unwrap();
        assert_eq!(embed_from_sidecar(&track, &cfg, false, None, true).unwrap(), SidecarEmbed::WouldEmbed(dir.join("missing.lrc")));
        assert_eq!(embed_from_sidecar(&track, &cfg, false, Some("txt"), true).unwrap(), SidecarEmbed::NoFile);

        let had = Track { has_lyrics: true, ..track };
        assert_eq!(embed_from_sidecar(&had, &cfg, false, None, false).unwrap(), SidecarEmbed::AlreadyHad);
        // overwrite on a file that is not audio: the write fails with an error, it does not panic
        assert!(embed_from_sidecar(&had, &cfg, true, None, false).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
