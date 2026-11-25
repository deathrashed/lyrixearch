//! Reading track metadata and embedding lyrics, via `lofty`.
//!
//! Plain lyrics: ID3v2 `USLT`, MP4 `©lyr`, Vorbis `LYRICS`, ASF `WM/Lyrics`, APE `Lyrics`.
//! Synced lyrics: ID3v2 `SYLT` and Vorbis `SYNCEDLYRICS`. MP3 and FLAC/Ogg are edited through
//! their concrete tag types so frames the generic tag cannot represent (such as `SYLT`) are
//! replaced instead of left behind; everything else in the file is preserved.

use crate::clean::guess_from_filename;
use crate::model::{Query, Track};
use anyhow::{Context, Result};
use lofty::config::{ParseOptions, WriteOptions};
use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::flac::FlacFile;
use lofty::id3::v2::{BinaryFrame, Frame, FrameFlags, FrameId, Id3v2Tag, SyncTextContentType, SynchronizedTextFrame, TimestampFormat, UnsynchronizedTextFrame};
use lofty::mpeg::MpegFile;
use lofty::ogg::tag::VorbisComments;
use lofty::ogg::{OpusFile, SpeexFile, VorbisFile};
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag, TagType};
use lofty::TextEncoding;
use std::path::Path;
use std::time::SystemTime;

/// Vorbis comment keys that hold lyrics (FLAC, Ogg Vorbis, Opus, Speex).
const VORBIS_PLAIN: &str = "LYRICS";
const VORBIS_UNSYNCED: &str = "UNSYNCEDLYRICS";
const VORBIS_SYNCED: &str = "SYNCEDLYRICS";

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
///
/// Same as [`write_lyrics_with`] without synced lyrics; any synced lyrics already in the file are
/// removed so they cannot disagree with the new text.
pub fn write_lyrics(path: &Path, text: &str, keep_mtime: bool) -> Result<()> {
    write_lyrics_with(path, text, None, keep_mtime)
}

/// Embed plain `text`, plus `synced` LRC where the format has a place for it:
/// ID3v2 `SYLT` (MP3) and the `SYNCEDLYRICS` Vorbis comment (FLAC, Ogg, Opus, Speex).
/// Other formats store only the plain text. Existing lyrics are replaced; every other tag
/// field and frame is kept.
pub fn write_lyrics_with(path: &Path, text: &str, synced: Option<&str>, keep_mtime: bool) -> Result<()> {
    let mtime = modified_time(path, keep_mtime);
    let synced = synced.filter(|s| crate::lrc::looks_synced(s));
    with_retries(path, || match file_type(path) {
        Some(FileType::Mpeg) => edit_mpeg(path, |tag| set_id3_lyrics(tag, Some(text), synced)),
        Some(ft) if is_vorbis(ft) => edit_vorbis(path, ft, |vc| set_vorbis_lyrics(vc, Some(text), synced)),
        _ => write_generic(path, text),
    })?;
    restore_mtime(path, mtime)
}

/// Remove embedded lyrics, plain and synced.
pub fn remove_lyrics(path: &Path, keep_mtime: bool) -> Result<()> {
    let mtime = modified_time(path, keep_mtime);
    with_retries(path, || match file_type(path) {
        Some(FileType::Mpeg) => edit_mpeg(path, |tag| set_id3_lyrics(tag, None, None)),
        Some(ft) if is_vorbis(ft) => edit_vorbis(path, ft, |vc| set_vorbis_lyrics(vc, None, None)),
        _ => remove_generic(path),
    })?;
    restore_mtime(path, mtime)
}

fn file_type(path: &Path) -> Option<FileType> {
    lofty::probe::Probe::open(path).ok()?.guess_file_type().ok()?.file_type()
}

fn is_vorbis(ft: FileType) -> bool {
    matches!(ft, FileType::Flac | FileType::Vorbis | FileType::Opus | FileType::Speex)
}

fn modified_time(path: &Path, keep: bool) -> Option<SystemTime> {
    if keep {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    } else {
        None
    }
}

/// Put the modified time back after a write. A failure here is reported: the caller asked for it.
fn restore_mtime(path: &Path, mtime: Option<SystemTime>) -> Result<()> {
    if let Some(t) = mtime {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .and_then(|f| f.set_modified(t))
            .with_context(|| format!("lyrics saved, but the modified time of {} could not be restored", path.display()))?;
    }
    Ok(())
}

/// A couple of attempts, like the original: transient locks on network shares are common.
fn with_retries(path: &Path, mut write: impl FnMut() -> Result<()>) -> Result<()> {
    let mut last_err = None;
    for attempt in 0..3 {
        match write() {
            Ok(()) => return Ok(()),
            Err(e) => {
                last_err = Some(e);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(250));
                }
            }
        }
    }
    Err(last_err.expect("three attempts were made")).with_context(|| format!("saving tags to {}", path.display()))
}

/// MP3: edit the ID3v2 tag directly so `USLT` gets a language and `SYLT` (which lofty's generic
/// tag cannot see) is replaced instead of left stale.
fn edit_mpeg(path: &Path, edit: impl FnOnce(&mut Id3v2Tag) -> Result<()>) -> Result<()> {
    let mut file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut mpeg = MpegFile::read_from(&mut file, ParseOptions::new()).with_context(|| format!("reading {}", path.display()))?;
    drop(file);
    if mpeg.id3v2().is_none() {
        mpeg.set_id3v2(Id3v2Tag::new());
    }
    edit(mpeg.id3v2_mut().context("no ID3v2 tag")?)?;
    mpeg.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// Replace every `USLT`/`SYLT` frame with the given text (`None` removes them).
fn set_id3_lyrics(tag: &mut Id3v2Tag, plain: Option<&str>, synced: Option<&str>) -> Result<()> {
    tag.retain(|f| !matches!(f.id_str(), "USLT" | "SYLT"));
    if let Some(text) = plain {
        tag.insert(Frame::UnsynchronizedText(UnsynchronizedTextFrame::new(TextEncoding::UTF8, *b"eng", "", text.to_string())));
    }
    if let Some(lines) = synced.map(crate::lrc::timed_lines).filter(|l| !l.is_empty()) {
        let sylt = SynchronizedTextFrame::new(TextEncoding::UTF8, *b"eng", TimestampFormat::MS, SyncTextContentType::Lyrics, None, lines);
        let bytes = sylt.as_bytes(WriteOptions::default()).context("encoding the SYLT frame")?;
        tag.insert(Frame::Binary(BinaryFrame::new(FrameId::Valid("SYLT".into()), bytes)));
    }
    Ok(())
}

/// FLAC and Ogg: edit the Vorbis comments directly so `SYNCEDLYRICS` can be written.
fn edit_vorbis(path: &Path, ft: FileType, edit: impl FnOnce(&mut VorbisComments)) -> Result<()> {
    let mut file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let opts = ParseOptions::new();
    let ctx = || format!("reading {}", path.display());
    match ft {
        FileType::Flac => {
            let mut f = FlacFile::read_from(&mut file, opts).with_context(ctx)?;
            drop(file);
            if f.vorbis_comments().is_none() {
                f.set_vorbis_comments(VorbisComments::default());
            }
            edit(f.vorbis_comments_mut().context("no Vorbis comments")?);
            f.save_to_path(path, WriteOptions::default())?;
        }
        FileType::Vorbis => {
            let mut f = VorbisFile::read_from(&mut file, opts).with_context(ctx)?;
            drop(file);
            edit(f.vorbis_comments_mut());
            f.save_to_path(path, WriteOptions::default())?;
        }
        FileType::Opus => {
            let mut f = OpusFile::read_from(&mut file, opts).with_context(ctx)?;
            drop(file);
            edit(f.vorbis_comments_mut());
            f.save_to_path(path, WriteOptions::default())?;
        }
        FileType::Speex => {
            let mut f = SpeexFile::read_from(&mut file, opts).with_context(ctx)?;
            drop(file);
            edit(f.vorbis_comments_mut());
            f.save_to_path(path, WriteOptions::default())?;
        }
        other => anyhow::bail!("{other:?} files do not use Vorbis comments"),
    }
    Ok(())
}

/// `LYRICS` holds the plain text and `SYNCEDLYRICS` the LRC; a stale `UNSYNCEDLYRICS` is dropped
/// so readers that prefer it do not show old lyrics. `None` removes all three.
fn set_vorbis_lyrics(vc: &mut VorbisComments, plain: Option<&str>, synced: Option<&str>) {
    for key in [VORBIS_PLAIN, VORBIS_UNSYNCED, VORBIS_SYNCED] {
        let _ = vc.remove(key).count();
    }
    if let Some(text) = plain {
        vc.insert(VORBIS_PLAIN.to_string(), text.to_string());
    }
    if let Some(lrc) = synced {
        vc.insert(VORBIS_SYNCED.to_string(), lrc.to_string());
    }
}

/// Every other format (MP4 `©lyr`, ASF `WM/Lyrics`, APE, ...) through lofty's generic tag.
fn write_generic(path: &Path, text: &str) -> Result<()> {
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
    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

fn remove_generic(path: &Path) -> Result<()> {
    let mut tagged = lofty::read_from_path(path).with_context(|| format!("opening {}", path.display()))?;
    let kinds: Vec<_> = tagged.tags().iter().map(|t| t.tag_type()).collect();
    for kind in kinds {
        if let Some(tag) = tagged.tag_mut(kind) {
            tag.remove_key(ItemKey::Lyrics);
            tag.remove_key(ItemKey::UnsyncLyrics);
        }
    }
    tagged.save_to_path(path, WriteOptions::default())?;
    Ok(())
}

/// Synced lyrics stored in the file (`SYLT` rebuilt as LRC, or `SYNCEDLYRICS`), if any.
pub fn read_synced(path: &Path) -> Option<String> {
    let ft = file_type(path)?;
    let mut file = std::fs::File::open(path).ok()?;
    let opts = ParseOptions::new();
    let vorbis = |vc: Option<&VorbisComments>| vc.and_then(|c| c.get(VORBIS_SYNCED)).map(str::to_string).filter(|s| !s.trim().is_empty());
    match ft {
        FileType::Mpeg => {
            let mpeg = MpegFile::read_from(&mut file, opts).ok()?;
            let Frame::Binary(bin) = mpeg.id3v2()?.get(&FrameId::Valid("SYLT".into()))? else { return None };
            let sylt = SynchronizedTextFrame::parse(&bin.data, FrameFlags::default()).ok()?;
            let lrc: Vec<String> = sylt
                .content
                .iter()
                .map(|(ms, text)| {
                    let (min, sec, cs) = (ms / 60_000, (ms / 1000) % 60, (ms % 1000) / 10);
                    format!("[{min:02}:{sec:02}.{cs:02}]{text}")
                })
                .collect();
            (!lrc.is_empty()).then(|| lrc.join("\n"))
        }
        FileType::Flac => vorbis(FlacFile::read_from(&mut file, opts).ok()?.vorbis_comments()),
        FileType::Vorbis => vorbis(Some(VorbisFile::read_from(&mut file, opts).ok()?.vorbis_comments())),
        FileType::Opus => vorbis(Some(OpusFile::read_from(&mut file, opts).ok()?.vorbis_comments())),
        FileType::Speex => vorbis(Some(SpeexFile::read_from(&mut file, opts).ok()?.vorbis_comments())),
        _ => None,
    }
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
