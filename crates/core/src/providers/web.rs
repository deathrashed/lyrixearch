//! Helpers shared by the HTML-scraping providers.

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;

pub const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// GET a page as text. Challenge pages and bot walls are reported as errors — we do not try
/// to get around them.
pub async fn get_html(http: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> Result<String> {
    let mut req = http.get(url).header("User-Agent", BROWSER_UA).header("Accept-Language", "en-US,en;q=0.9");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if status.as_u16() == 429 {
        return Err(super::RateLimited::from_response(&resp, "rate limited (HTTP 429)").into());
    }
    let retry_hint = resp.headers().contains_key(reqwest::header::RETRY_AFTER).then(|| super::RateLimited::from_response(&resp, "busy (HTTP 503)"));
    let body = resp.text().await.unwrap_or_default();
    if body.contains("_cf_chl_opt") || body.contains("cf-challenge") {
        return Err(super::Challenge { url: url.to_string(), reason: "blocked by a Cloudflare challenge".into() }.into());
    }
    if status.as_u16() == 503 {
        // busy: honour Retry-After when sent, otherwise back off for a minute
        return Err(retry_hint.unwrap_or(super::RateLimited { retry_after: std::time::Duration::from_secs(60), reason: "busy (HTTP 503)".into() }).into());
    }
    if matches!(status.as_u16(), 401 | 403) {
        bail!("blocked (HTTP {status})");
    }
    if status.as_u16() == 404 {
        return Ok(String::new());
    }
    if !status.is_success() {
        bail!("HTTP {status}");
    }
    Ok(body)
}

static BR: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)<br\s*/?>").unwrap());
static BLOCK_END: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)</(p|div|h\d|li)\s*>|<(p|div|h\d|li)(\s[^>]*)?>").unwrap());
static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());
static BLANKS: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

/// HTML fragment -> plain text with line breaks preserved.
pub fn html_to_text(html: &str) -> String {
    let s = BR.replace_all(html, "\n");
    let s = BLOCK_END.replace_all(&s, "\n");
    let s = TAG.replace_all(&s, "");
    let s = html_escape::decode_html_entities(&s).replace('\u{a0}', " ");
    let lines: Vec<&str> = s.lines().map(str::trim_end).collect();
    BLANKS.replace_all(lines.join("\n").trim(), "\n\n").into_owned()
}

/// Return the inner HTML of the `<div>` whose opening tag ends at `open_end`, honouring nesting.
pub fn balanced_div(html: &str, open_end: usize) -> &str {
    let rest = &html[open_end..];
    let mut depth = 1usize;
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Compare raw bytes: `rest` may contain multi-byte characters right after the tag name.
            let tail = &bytes[i..];
            if tail.len() > 4 && tail[..4].eq_ignore_ascii_case(b"<div") && matches!(tail[4], b' ' | b'>' | b'\n' | b'\t') {
                depth += 1;
            } else if tail.len() >= 6 && tail[..6].eq_ignore_ascii_case(b"</div>") {
                depth -= 1;
                if depth == 0 {
                    return &rest[..i];
                }
            }
        }
        i += 1;
    }
    rest
}

pub fn first_group<'a>(re: &Regex, s: &'a str) -> Option<&'a str> {
    re.captures(s).and_then(|c| c.get(1)).map(|m| m.as_str())
}

/// "Artist – Title" (or hyphen) split, used on page titles.
pub fn split_artist_title(s: &str) -> Option<(String, String)> {
    for sep in [" – ", " — ", " - "] {
        if let Some((a, t)) = s.split_once(sep) {
            return Some((a.trim().to_string(), t.trim().to_string()));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_conversion() {
        assert_eq!(html_to_text("a<br/>b &amp; c<div>x</div>"), "a\nb & c\nx");
    }

    #[test]
    fn balanced() {
        let h = "<div id=a>one<div>two</div>three</div>after";
        let open_end = h.find('>').unwrap() + 1;
        assert_eq!(balanced_div(h, open_end), "one<div>two</div>three");
    }
}
