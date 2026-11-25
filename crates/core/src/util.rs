//! Small OS helpers shared by the front ends.

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Absolute audio path used for pending results across front ends and working directories.
pub fn pending_key(path: &Path) -> PathBuf {
    path.canonicalize().or_else(|_| std::path::absolute(path)).unwrap_or_else(|_| path.to_path_buf())
}

/// Copy a plugin directory without following links or copying Python caches.
pub fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    if !fs::symlink_metadata(from)?.is_dir() {
        bail!("plugin source is not a directory: {}", from.display());
    }
    let from = from.canonicalize()?;
    let to = resolve_destination(&std::path::absolute(to)?)?;
    if to.starts_with(&from) {
        bail!("cannot copy {} into itself: {}", from.display(), to.display());
    }
    copy_contents(&from, &to)
}

/// Resolve existing ancestors first so an absent destination can be checked before creation.
fn resolve_destination(path: &Path) -> Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(path.canonicalize()?),
        Ok(_) => bail!("plugin destination is not a directory: {}", path.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            let parent = path.parent().context("destination has no existing ancestor")?;
            let name = path.file_name().context("destination must name a directory")?;
            Ok(resolve_destination(parent)?.join(name))
        }
        Err(e) => Err(e).with_context(|| format!("checking {}", path.display())),
    }
}

fn copy_contents(from: &Path, to: &Path) -> Result<()> {
    match fs::symlink_metadata(to) {
        Ok(metadata) if metadata.is_dir() => {}
        Ok(_) => bail!("plugin destination is not a directory: {}", to.display()),
        Err(e) if e.kind() == ErrorKind::NotFound => fs::create_dir_all(to)?,
        Err(e) => return Err(e.into()),
    }
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == "__pycache__" {
            continue;
        }
        let src = entry.path();
        let dst = to.join(name);
        let kind = fs::symlink_metadata(&src)?.file_type();
        if kind.is_dir() {
            copy_contents(&src, &dst)?;
        } else if kind.is_file() {
            match fs::symlink_metadata(&dst) {
                Ok(metadata) if metadata.is_file() => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        if metadata.nlink() > 1 {
                            bail!("will not overwrite a hard-linked plugin file: {}", dst.display());
                        }
                    }
                }
                Ok(_) => bail!("plugin destination is not a regular file: {}", dst.display()),
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            fs::copy(&src, &dst).with_context(|| format!("copying {} to {}", src.display(), dst.display()))?;
        }
    }
    Ok(())
}

/// Web search engines offered for "search in browser" (same set as the original).
pub const SEARCH_ENGINES: &[&str] = &["google", "google_ai", "brave_ai", "bing", "duckduckgo", "yahoo", "yandex"];

/// Search URL for `"artist - title lyrics"` on the chosen engine. `google_ai` is Google's AI Mode
/// and `brave_ai` is Brave's "Ask"; both take the query in the URL, like the plain engines.
pub fn search_url(engine: &str, artist: &str, title: &str) -> String {
    let q = urlencoding::encode(&format!("{artist} - {title} lyrics")).replace("%20", "+");
    match engine.to_ascii_lowercase().as_str() {
        "google_ai" => format!("https://www.google.com/search?udm=50&source=searchlabs&q={q}"),
        "brave_ai" => format!("https://search.brave.com/ask?q={q}"),
        "bing" => format!("https://www.bing.com/search?q={q}"),
        "duckduckgo" | "ddg" => format!("https://duckduckgo.com/?q={q}"),
        "yahoo" => format!("https://search.yahoo.com/search?p={q}"),
        "yandex" => format!("https://yandex.com/search/?text={q}"),
        _ => format!("https://www.google.com/search?q={q}"),
    }
}

/// Open a URL in the default browser. Only http(s) URLs are accepted.
pub fn open_url(url: &str) -> std::io::Result<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "only http(s) URLs can be opened"));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = Command::new("open");
    // Not `cmd /C start`: cmd treats `&` in a URL as a command separator.
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    };
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let mut cmd = Command::new("xdg-open");
    cmd.arg(url).spawn().map(|_| ())
}

/// Show a file in the system file browser (Finder, Explorer, or the folder in the default manager).
pub fn reveal(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    return Command::new("open").arg("-R").arg(path).spawn().map(|_| ());
    #[cfg(target_os = "windows")]
    return Command::new("explorer").arg(format!("/select,{}", path.display())).spawn().map(|_| ());
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    return Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn().map(|_| ());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_plugin_without_recursing_into_itself_or_following_links() -> Result<()> {
        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
        let dir = std::env::temp_dir().join(format!("lyrixearch-copy-{}-{stamp}", std::process::id()));
        let src = dir.join("plugin");
        fs::create_dir_all(src.join("__pycache__"))?;
        fs::create_dir(src.join("data"))?;
        fs::write(src.join("data/plugin.txt"), "lyrics")?;
        assert!(copy_dir(&src, &src.join(".")).is_err());
        assert!(copy_dir(&src, &src.join("missing/nested")).is_err());
        assert!(!src.join("missing").exists());
        #[cfg(unix)]
        std::os::unix::fs::symlink(&src, src.join("loop"))?;
        let dst = dir.join("installed/plugin");
        copy_dir(&src, &dst)?;
        assert_eq!(fs::read_to_string(dst.join("data/plugin.txt"))?, "lyrics");
        assert!(!dst.join("__pycache__").exists());
        assert!(!dst.join("loop").exists());
        #[cfg(unix)]
        {
            let other = dir.join("other");
            fs::create_dir(&other)?;
            let linked_dst = dir.join("linked");
            fs::create_dir(&linked_dst)?;
            std::os::unix::fs::symlink(&other, linked_dst.join("data"))?;
            assert!(copy_dir(&src, &linked_dst).is_err());
            assert!(!other.join("plugin.txt").exists());
        }
        fs::remove_dir_all(dir)?;
        Ok(())
    }

    #[test]
    fn urls() {
        assert_eq!(search_url("bing", "A B", "C"), "https://www.bing.com/search?q=A+B+-+C+lyrics");
        assert_eq!(
            search_url("google_ai", "Queen", "Bohemian Rhapsody"),
            "https://www.google.com/search?udm=50&source=searchlabs&q=Queen+-+Bohemian+Rhapsody+lyrics"
        );
        assert_eq!(search_url("brave_ai", "Queen", "Song"), "https://search.brave.com/ask?q=Queen+-+Song+lyrics");
        assert!(open_url("file:///etc/passwd").is_err());
    }
}
