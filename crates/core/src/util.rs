//! Small OS helpers shared by the front ends.

use std::process::Command;

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
