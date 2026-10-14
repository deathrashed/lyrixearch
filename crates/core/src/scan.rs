use std::path::{Path, PathBuf};
use walkdir::WalkDir;

pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "mp4", "aac", "ogg", "oga", "opus", "wma", "wav", "aif", "aiff", "ape", "wv"];

/// Folders that hold no music of yours: Finder/Spotlight/trash bookkeeping on external drives.
fn is_junk_dir(name: &str) -> bool {
    matches!(name, ".Trashes" | ".Spotlight-V100" | ".fseventsd" | ".TemporaryItems" | "$RECYCLE.BIN" | "System Volume Information" | "__MACOSX")
}

/// `._Song.mp3` AppleDouble stubs that macOS writes next to files on FAT/exFAT/NTFS volumes.
fn is_junk_file(name: &str) -> bool {
    name.starts_with("._")
}

pub fn is_audio(path: &Path) -> bool {
    path.extension().map(|e| AUDIO_EXTENSIONS.contains(&e.to_string_lossy().to_lowercase().as_str())).unwrap_or(false)
}

/// Expand files and folders into a sorted, de-duplicated list of audio files.
pub fn collect_audio(paths: &[PathBuf], recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_dir() {
            let walker = WalkDir::new(p)
                .max_depth(if recursive { usize::MAX } else { 1 })
                .follow_links(true)
                .into_iter()
                .filter_entry(|e| !(e.file_type().is_dir() && e.depth() > 0 && is_junk_dir(&e.file_name().to_string_lossy())));
            for e in walker.filter_map(Result::ok) {
                if e.file_type().is_file() && is_audio(e.path()) && !is_junk_file(&e.file_name().to_string_lossy()) {
                    out.push(e.into_path());
                }
            }
        } else if p.is_file() && is_audio(p) {
            out.push(p.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_nested_music_and_skips_drive_junk() {
        let root = std::env::temp_dir().join(format!("lf-scan-{}", std::process::id()));
        let make = |rel: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, b"x").unwrap();
        };
        make("A/Band/1999 - First/01.mp3");
        make("A/Band/2003 - Second/CD1/deep/02.FLAC");
        make("A/Band/2003 - Second/CD1/deep/._02.FLAC");
        make("A/Band/notes.txt");
        make("A/.Trashes/501/gone.mp3");
        make("A/__MACOSX/Band/03.mp3");
        let found = collect_audio(&[root.join("A")], true);
        let names: Vec<_> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["01.mp3", "02.FLAC"], "{names:?}");
        // non-recursive only looks at the top level
        assert!(collect_audio(&[root.join("A")], false).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
