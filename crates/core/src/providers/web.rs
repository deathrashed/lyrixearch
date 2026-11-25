//! Helpers shared by the HTML-scraping providers.

use anyhow::{bail, Context, Result};
use once_cell::sync::Lazy;
use regex::Regex;

pub const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// A real browser engine that can load pages a plain HTTP client is turned away from. Cloudflare's
/// bot checks also fingerprint the connection itself, so even a passed check's cookie does not let
/// an HTTP client in; a browser window that passed the check can keep loading that site's pages.
/// The desktop app registers its hidden browser with [`set_browser`]; the CLI has none and gets
/// [`super::Challenge`] errors instead.
#[async_trait::async_trait]
pub trait BrowserFetch: Send + Sync {
    /// The page at `url`: its HTML, or with `text` its visible text (for JSON endpoints).
    /// A check that needs a person should come back as a [`super::Challenge`] error.
    async fn fetch(&self, url: &str, text: bool) -> Result<String>;
}

static BROWSER: once_cell::sync::OnceCell<std::sync::Arc<dyn BrowserFetch>> = once_cell::sync::OnceCell::new();
/// Sites that answered a plain request with a bot check: their pages go through the browser from then on.
static BROWSER_HOSTS: Lazy<std::sync::RwLock<std::collections::HashSet<String>>> = Lazy::new(Default::default);

/// Register the browser used for sites behind bot checks (once per process).
pub fn set_browser(browser: std::sync::Arc<dyn BrowserFetch>) {
    let _ = BROWSER.set(browser);
}

fn host_of(url: &str) -> String {
    let host = url.split('/').nth(2).unwrap_or("").split(':').next().unwrap_or("").to_ascii_lowercase();
    host.strip_prefix("www.").map(str::to_string).unwrap_or(host)
}

/// Mark `url`'s site as one that needs the browser (also used by tests).
pub fn route_through_browser(url: &str) {
    BROWSER_HOSTS.write().unwrap().insert(host_of(url));
}

/// GET a page's HTML. A site that answers with a bot check is loaded through the registered
/// browser instead (see [`BrowserFetch`]); without one the check is reported as an error.
pub async fn get_html(http: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> Result<String> {
    get(http, url, headers, false).await
}

/// Like [`get_html`], for endpoints that answer with text such as JSON.
pub async fn get_text(http: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> Result<String> {
    get(http, url, headers, true).await
}

async fn get(http: &reqwest::Client, url: &str, headers: &[(&str, &str)], text: bool) -> Result<String> {
    let browser = BROWSER.get();
    if let Some(b) = browser {
        if BROWSER_HOSTS.read().unwrap().contains(&host_of(url)) {
            return b.fetch(url, text).await;
        }
    }
    match plain_get(http, url, headers).await {
        Err(e) if e.downcast_ref::<super::Challenge>().is_some() => match browser {
            Some(b) => {
                route_through_browser(url);
                b.fetch(url, text).await
            }
            None => Err(e),
        },
        other => other,
    }
}

/// GET a page over plain HTTP. Challenge pages and bot walls are reported as errors.
async fn plain_get(http: &reqwest::Client, url: &str, headers: &[(&str, &str)]) -> Result<String> {
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
    // a body that cannot be read is a failed request, not an empty page ("no match")
    let body = resp.text().await.with_context(|| format!("reading {url}"))?;
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

/// Fold common Latin accents to plain ASCII letters ("Mötley Crüe" -> "Motley Crue"), as lyrics
/// sites do when they build URLs from names.
pub fn ascii_fold(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
            'À' | 'Á' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'A',
            'è' | 'é' | 'ê' | 'ë' => 'e',
            'È' | 'É' | 'Ê' | 'Ë' => 'E',
            'ì' | 'í' | 'î' | 'ï' => 'i',
            'Ì' | 'Í' | 'Î' | 'Ï' => 'I',
            'ò' | 'ó' | 'ô' | 'ö' | 'õ' | 'ø' => 'o',
            'Ò' | 'Ó' | 'Ô' | 'Ö' | 'Õ' | 'Ø' => 'O',
            'ù' | 'ú' | 'û' | 'ü' => 'u',
            'Ù' | 'Ú' | 'Û' | 'Ü' => 'U',
            'ñ' => 'n',
            'Ñ' => 'N',
            'ç' => 'c',
            'Ç' => 'C',
            'ý' | 'ÿ' => 'y',
            other => other,
        })
        .collect()
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
    fn folds_accents() {
        assert_eq!(ascii_fold("Mötley Crüe"), "Motley Crue");
        assert_eq!(ascii_fold("Beyoncé, Sigur Rós, Ñu"), "Beyonce, Sigur Ros, Nu");
        assert_eq!(ascii_fold("夜に駆ける"), "夜に駆ける");
    }

    struct FakeBrowser;

    #[async_trait::async_trait]
    impl BrowserFetch for FakeBrowser {
        async fn fetch(&self, url: &str, text: bool) -> Result<String> {
            Ok(format!("browser:{url}:{text}"))
        }
    }

    #[tokio::test]
    async fn sites_behind_checks_go_through_the_browser() {
        set_browser(std::sync::Arc::new(FakeBrowser));
        route_through_browser("https://www.checked.example/x");
        // no network involved: the site is known to need the browser
        let http = reqwest::Client::new();
        assert_eq!(get_html(&http, "https://checked.example/song", &[]).await.unwrap(), "browser:https://checked.example/song:false");
        assert_eq!(get_text(&http, "https://www.checked.example/api", &[]).await.unwrap(), "browser:https://www.checked.example/api:true");
        assert_eq!(host_of("https://WWW.Example.org:443/a"), "example.org");
    }

    #[test]
    fn balanced() {
        let h = "<div id=a>one<div>two</div>three</div>after";
        let open_end = h.find('>').unwrap() + 1;
        assert_eq!(balanced_div(h, open_end), "one<div>two</div>three");
    }
}
