//! Reading track metadata and embedding lyrics, via `lofty` (ID3v2 `USLT`, MP4 `©lyr`,
//! Vorbis `LYRICS`, ASF `WM/Lyrics`, APE — lofty maps `ItemKey::Lyrics` per format).

use crate::clean::guess_from_filename;
use crate::model::{Query, Track};
use anyhow::{Context, Result};
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use std::path::Path;
use std::time::SystemTime;

/// ID3v2 only accepts `UnsyncLyrics` (the `USLT` frame); other formats take `Lyrics`
/// (`©lyr`, `LYRICS`, `WM/Lyrics`, ...).
fn write_key(tag: &Tag) -> ItemKey {
    if tag.tag_type() == TagType::Id3v2 {
        ItemKey::UnsyncLyrics
    } else {
        ItemKey::Lyrics
    }
}

/// Embedded lyrics from either key, whichever the format stores.
fn tag_lyrics(tag: &Tag) -> Option<&str> {
    [ItemKey::Lyrics, ItemKey::UnsyncLyrics].into_iter().find_map(|k| tag.get_string(k)).filter(|s| !s.trim().is_empty())
}

pub fn read_track(path: &Path) -> Result<Track> {
    let tagged = lofty::read_from_path(path).with_context(|| format!("reading tags from {}", path.display()))?;
    let duration = tagged.properties().duration().as_secs() as u32;
    let format = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();

    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let track_number = tag.and_then(|t| t.track());
    let (mut artist, mut title, album, album_artist, has_lyrics) = match tag {
        Some(t) => (
            t.artist().map(|s| s.to_string()).unwrap_or_default(),
            t.title().map(|s| s.to_string()).unwrap_or_default(),
            t.album().map(|s| s.to_string()),
            t.get_string(ItemKey::AlbumArtist).map(str::to_string).filter(|s| !s.trim().is_empty()),
            tag_lyrics(t).is_some(),
        ),
        None => (String::new(), String::new(), None, None, false),
    };

    if artist.trim().is_empty() || title.trim().is_empty() {
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        if let Some((a, t)) = guess_from_filename(&stem) {
            if artist.trim().is_empty() {
                artist = a;
            }
            if title.trim().is_empty() {
                title = t;
            }
        } else if title.trim().is_empty() {
            title = stem;
        }
    }

    Ok(Track {
        path: path.to_path_buf(),
        query: Query {
            artist,
            title,
            album,
            album_artist,
            track_number,
            duration_secs: (duration > 0).then_some(duration),
            source_path: Some(path.to_string_lossy().into_owned()),
        },
        has_lyrics,
        format,
    })
}

/// Embed `text` as the track's lyrics. Optionally restores the modified time afterwards.
pub fn write_lyrics(path: &Path, text: &str, keep_mtime: bool) -> Result<()> {
    let mtime: Option<SystemTime> = if keep_mtime { std::fs::metadata(path).and_then(|m| m.modified()).ok() } else { None };

    let mut tagged = lofty::read_from_path(path).with_context(|| format!("opening {}", path.display()))?;
    if tagged.primary_tag().is_none() && tagged.first_tag().is_none() {
        let ty = tagged.primary_tag_type();
        tagged.insert_tag(Tag::new(ty));
    }
    let tag = if tagged.primary_tag().is_some() { tagged.primary_tag_mut() } else { tagged.first_tag_mut() }.context("no writable tag")?;
    let key = write_key(tag);
    if !tag.insert_text(key, text.to_string()) {
        anyhow::bail!("{:?} tags cannot store lyrics", tag.tag_type());
    }

    // A couple of attempts, like the original: transient locks on network shares are common.
    let mut last_err = None;
    for _ in 0..3 {
        match tagged.save_to_path(path, lofty::config::WriteOptions::default()) {
            Ok(()) => {
                last_err = None;
                break;
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
    if let Some(e) = last_err {
        return Err(e).with_context(|| format!("saving tags to {}", path.display()));
    }

    if let Some(t) = mtime {
        let f = std::fs::OpenOptions::new().write(true).open(path)?;
        f.set_modified(t).ok();
    }
    Ok(())
}

/// Remove embedded lyrics.
pub fn remove_lyrics(path: &Path, keep_mtime: bool) -> Result<()> {
    let mtime = if keep_mtime { std::fs::metadata(path).and_then(|m| m.modified()).ok() } else { None };
    let mut tagged = lofty::read_from_path(path)?;
    let kinds: Vec<_> = tagged.tags().iter().map(|t| t.tag_type()).collect();
    for kind in kinds {
        if let Some(tag) = tagged.tag_mut(kind) {
            tag.remove_key(ItemKey::Lyrics);
            tag.remove_key(ItemKey::UnsyncLyrics);
        }
    }
    tagged.save_to_path(path, lofty::config::WriteOptions::default())?;
    if let Some(t) = mtime {
        std::fs::OpenOptions::new().write(true).open(path)?.set_modified(t).ok();
    }
    Ok(())
}

/// Raw bytes of the embedded cover (front cover preferred, otherwise the first picture).
pub fn read_cover(path: &Path) -> Option<Vec<u8>> {
    use lofty::picture::PictureType;
    let tagged = lofty::read_from_path(path).ok()?;
    for tag in tagged.tags() {
        let pics = tag.pictures();
        let pic = pics.iter().find(|p| p.pic_type() == PictureType::CoverFront).or_else(|| pics.first());
        if let Some(p) = pic {
            return Some(p.data().to_vec());
        }
    }
    None
}

/// Embedded lyrics text, if any.
pub fn read_lyrics(path: &Path) -> Option<String> {
    let tagged = lofty::read_from_path(path).ok()?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag())?;
    tag_lyrics(tag).map(str::to_string)
}
