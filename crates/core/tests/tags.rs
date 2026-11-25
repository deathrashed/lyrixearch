//! Tag writing against real (tiny) audio files. Each test works on a copy in a temp folder.
//!
//! `rich.mp3` carries the frames a real library has (cover, PRIV, GEOB, POPM, TXXX, UFID,
//! COMM, an old SYLT and two old USLT frames); `rich.flac` has a custom field, ReplayGain and
//! stale lyric fields. Writing lyrics must replace the lyrics and leave everything else alone.

use lofty::config::ParseOptions;
use lofty::file::AudioFile;
use lofty::mpeg::MpegFile;
use lyrixearch_core::tags;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const LRC: &str = "[ar:Artist]\n[00:01.00]first line\n[00:02.50]second line";

fn fixture(name: &str) -> (tempdir::Dir, PathBuf) {
    let dir = tempdir::Dir::new(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name);
    let dst = dir.path().join(name);
    std::fs::copy(&src, &dst).unwrap();
    (dir, dst)
}

fn frame_ids(path: &Path) -> Vec<String> {
    let mut f = std::fs::File::open(path).unwrap();
    let mpeg = MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
    let mut ids: Vec<String> = mpeg.id3v2().unwrap().into_iter().map(|fr| fr.id_str().to_string()).collect();
    ids.sort();
    ids
}

#[test]
fn mp3_replaces_lyrics_and_keeps_every_other_frame() {
    let (_dir, path) = fixture("rich.mp3");
    let before: Vec<String> = frame_ids(&path).into_iter().filter(|id| id != "USLT" && id != "SYLT").collect();

    tags::write_lyrics_with(&path, "first line\nsecond line", Some(LRC), false).unwrap();

    let after = frame_ids(&path);
    let others: Vec<String> = after.iter().filter(|id| *id != "USLT" && *id != "SYLT").cloned().collect();
    assert_eq!(others, before, "non-lyric frames must survive untouched");
    assert_eq!(after.iter().filter(|id| *id == "USLT").count(), 1, "old USLT frames are replaced, not kept beside the new one");
    assert_eq!(after.iter().filter(|id| *id == "SYLT").count(), 1, "the old SYLT is replaced");

    let mut f = std::fs::File::open(&path).unwrap();
    let mpeg = MpegFile::read_from(&mut f, ParseOptions::new()).unwrap();
    let uslt = mpeg.id3v2().unwrap().unsync_text().next().unwrap();
    assert_eq!(&uslt.language, b"eng");
    assert_eq!(uslt.content, "first line\nsecond line");

    assert_eq!(tags::read_lyrics(&path).as_deref(), Some("first line\nsecond line"));
    assert_eq!(tags::read_synced(&path).as_deref(), Some("[00:01.00]first line\n[00:02.50]second line"));
}

#[test]
fn mp3_plain_write_drops_a_stale_sylt() {
    let (_dir, path) = fixture("rich.mp3");
    assert!(tags::read_synced(&path).is_some(), "fixture starts with an old SYLT");
    tags::write_lyrics(&path, "only plain now", false).unwrap();
    assert_eq!(tags::read_synced(&path), None);
    assert!(!frame_ids(&path).contains(&"SYLT".to_string()));
}

#[test]
fn mp3_remove_clears_plain_and_synced() {
    let (_dir, path) = fixture("rich.mp3");
    tags::remove_lyrics(&path, false).unwrap();
    let ids = frame_ids(&path);
    assert!(!ids.iter().any(|id| id == "USLT" || id == "SYLT"));
    assert!(ids.contains(&"APIC".to_string()) && ids.contains(&"PRIV".to_string()) && ids.contains(&"GEOB".to_string()));
    assert_eq!(tags::read_lyrics(&path), None);
}

#[test]
fn flac_writes_lyrics_and_synced_and_keeps_custom_fields() {
    let (_dir, path) = fixture("rich.flac");
    tags::write_lyrics_with(&path, "first line\nsecond line", Some(LRC), false).unwrap();

    let mut f = std::fs::File::open(&path).unwrap();
    let flac = lofty::flac::FlacFile::read_from(&mut f, ParseOptions::new()).unwrap();
    let vc = flac.vorbis_comments().unwrap();
    assert_eq!(vc.get("LYRICS"), Some("first line\nsecond line"));
    assert_eq!(vc.get("SYNCEDLYRICS"), Some(LRC));
    assert_eq!(vc.get("UNSYNCEDLYRICS"), None, "a stale UNSYNCEDLYRICS must not linger");
    assert_eq!(vc.get("CUSTOMFIELD"), Some("keepme"));
    assert_eq!(vc.get("REPLAYGAIN_TRACK_GAIN"), Some("-6.00 dB"));
    assert_eq!(vc.get("ARTIST"), Some("Artist"));

    tags::remove_lyrics(&path, false).unwrap();
    assert_eq!(tags::read_lyrics(&path), None);
    assert_eq!(tags::read_synced(&path), None);
}

#[test]
fn ogg_opus_and_m4a_round_trip() {
    for (name, synced_stored) in [("plain.ogg", true), ("plain.opus", true), ("plain.m4a", false)] {
        let (_dir, path) = fixture(name);
        tags::write_lyrics_with(&path, "hello\nworld", Some(LRC), false).unwrap_or_else(|e| panic!("{name}: {e:#}"));
        assert_eq!(tags::read_lyrics(&path).as_deref(), Some("hello\nworld"), "{name}");
        assert_eq!(tags::read_synced(&path).is_some(), synced_stored, "{name}");
        let track = tags::read_track(&path).unwrap();
        assert!(track.has_lyrics, "{name}");
        assert_eq!(track.query.artist, "Artist", "{name}");
    }
}

#[test]
fn keep_mtime_restores_the_modified_time() {
    let (_dir, path) = fixture("rich.flac");
    let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    std::fs::OpenOptions::new().write(true).open(&path).unwrap().set_modified(old).unwrap();
    tags::write_lyrics(&path, "some words here", true).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);
}

#[test]
fn unsupported_file_is_an_error_not_a_panic() {
    let dir = tempdir::Dir::new("bad");
    let path = dir.path().join("not-audio.mp3");
    std::fs::write(&path, b"this is not audio").unwrap();
    assert!(tags::write_lyrics(&path, "x", false).is_err());
    assert!(tags::read_track(&path).is_err());
}

/// Minimal temp-dir helper (no extra dependency): removed on drop.
mod tempdir {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};

    pub struct Dir(PathBuf);

    impl Dir {
        pub fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!("lyrixearch-test-{}-{}-{n}", std::process::id(), tag.replace('.', "_")));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }

        pub fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
