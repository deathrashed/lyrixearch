//! Whole-file writes that never leave a half-written file behind.

use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;

/// Write `bytes` to `path` through a temporary file in the same folder, then rename it into place.
/// A crash, a full disk or a killed process leaves either the old file or the new one, never a mix.
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = path.file_name().context("path has no file name")?.to_string_lossy();
    // process id plus a counter: unique across processes and across threads of this one
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.tmp-{}-{seq}", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        // best effort: do not leave the temporary file around after a failure
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn replaces_content_and_leaves_no_temp_file() {
        let dir = std::env::temp_dir().join(format!("lyrixearch-atomic-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.txt");
        super::write(&p, b"one").unwrap();
        super::write(&p, b"two").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
        assert!(super::write(&dir.join("missing-dir").join("y.txt"), b"z").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
