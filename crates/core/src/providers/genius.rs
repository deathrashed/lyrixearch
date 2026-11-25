//! Genius. Lyrics are read from the public song page over plain HTTP; the song URL comes from
//! a guessed slug first and Genius's public search endpoint second. Challenge pages are
//! reported as "blocked" rather than bypassed.

use super::web::{balanced_div, get_html, html_to_text, split_artist_title};
use super::CheckStatus;
use super::Provider;
use crate::clean::{clean_title, strip_featuring};
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::Value;

pub struct Genius;

static CONTAINER: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div[^>]*data-lyrics-container="true"[^>]*>"#).unwrap());
static OG_TITLE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"og:title"\s+content="([^"]*)""#).unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://(www\.)?genius\.com/[^/?#]+-lyrics/?$").unwrap());
static NON_ALNUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^a-z0-9]+").unwrap());

/// "Beyoncé & Jay-Z" -> "beyonce-and-jay-z" style slug used in genius.com URLs.
pub fn slugify(s: &str) -> String {
    let s = s.to_lowercase().replace(['\'', '’', '.'], "").replace('&', " and ");
    let ascii: String = s.chars().map(fold).collect();
    let slug = NON_ALNUM.replace_all(&ascii, "-");
    slug.trim_matches('-').to_string()
}

fn fold(c: char) -> char {
    match c {
        'à' | 'á' | 'â' | 'ä' | 'ã' | 'å' => 'a',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ò' | 'ó' | 'ô' | 'ö' | 'õ' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

static EXCLUDED: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div[^>]*data-exclude-from-selection="true"[^>]*>"#).unwrap());

/// Genius puts the contributors/translations header, "Read More" and ads inside the lyrics container and marks
/// them `data-exclude-from-selection`; cut those blocks out before turning the rest into text.
fn without_excluded(html: &str) -> String {
    let mut out = html.to_string();
    // each pass removes one block; nested blocks disappear with their parent
    while let Some(m) = EXCLUDED.find(&out) {
        let inner_len = balanced_div(&out, m.end()).len();
        let end = (m.end() + inner_len + "</div>".len()).min(out.len());
        out.replace_range(m.start()..end, "");
    }
    out
}

fn extract_lyrics(html: &str) -> Option<String> {
    let parts: Vec<String> =
        CONTAINER.find_iter(html).map(|m| html_to_text(&without_excluded(balanced_div(html, m.end())))).filter(|t| !t.trim().is_empty()).collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

async fn read_page(http: &reqwest::Client, url: &str, q: &Query, cfg: &Config, min_check: bool) -> Result<Option<Lyrics>> {
    let html = get_html(http, url, &[]).await?;
    if html.is_empty() {
        return Ok(None);
    }
    let og = OG_TITLE.captures(&html).map(|c| html_escape::decode_html_entities(&c[1]).into_owned()).unwrap_or_default();
    let (artist, title) = split_artist_title(&og).unwrap_or_default();
    let sc = score(q, &artist, &title, None);
    if min_check && !sc.accepted(cfg.min_score) {
        return Ok(None);
    }
    Ok(extract_lyrics(&html).map(|text| Lyrics {
        plain: Some(text),
        source: "genius".into(),
        url: Some(url.to_string()),
        matched_artist: artist,
        matched_title: title,
        score: sc.overall,
        ..Default::default()
    }))
}

#[async_trait]
impl Provider for Genius {
    fn id(&self) -> &'static str {
        "genius"
    }

    fn supports_url(&self, url: &str) -> bool {
        URL_RE.is_match(url)
    }

    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        read_page(http, url, q, cfg, true).await
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        // 1) straight to the guessed slug
        let slug = slugify(&format!("{} {}", strip_featuring(&q.artist), clean_title(&strip_featuring(&q.title))));
        if !slug.is_empty() {
            let mut chars = slug.chars();
            let cap: String = chars.next().map(|c| c.to_ascii_uppercase()).into_iter().chain(chars).collect();
            let url = format!("https://genius.com/{cap}-lyrics");
            if let Some(l) = read_page(http, &url, q, cfg, true).await? {
                return Ok(Some(l));
            }
        }

        // 2) ask Genius's public search for the real URL
        let resp: Value = http
            .get("https://genius.com/api/search/multi")
            .header("User-Agent", super::web::BROWSER_UA)
            .query(&[("q", format!("{} {}", q.artist, q.title)), ("per_page", "5".into())])
            .send()
            .await
            .context("genius search")?
            .check_status("genius")?
            .json()
            .await
            .context("genius search: bad json")?;
        let sections = resp.pointer("/response/sections").and_then(Value::as_array).cloned().unwrap_or_default();
        let best = sections
            .iter()
            .flat_map(|s| s.get("hits").and_then(Value::as_array).cloned().unwrap_or_default())
            .filter(|h| h.get("type").and_then(Value::as_str) == Some("song"))
            .filter_map(|h| {
                let r = h.get("result")?;
                let title = r.get("title")?.as_str()?.to_string();
                let artist = r.pointer("/primary_artist/name").and_then(Value::as_str).unwrap_or("").to_string();
                let url = r.get("url")?.as_str()?.to_string();
                let sc = score(q, &artist, &title, None);
                sc.accepted(cfg.min_score).then_some((sc.overall, url))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        match best {
            Some((_, url)) => read_page(http, &url, q, cfg, false).await,
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_blocks_are_not_lyrics() {
        let html = r#"<div data-lyrics-container="true" class="c"><div data-exclude-from-selection="true" class="h"><button><span>3 Contributors</span></button><div><div>Translations</div></div><h1>Song Lyrics</h1></div>First line<br/>Second line<br/><br/>Third line<div data-exclude-from-selection="true">Read More</div></div>"#;
        assert_eq!(extract_lyrics(html).unwrap(), "First line\nSecond line\n\nThird line");
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("Beyoncé & Jay-Z"), "beyonce-and-jay-z");
        assert_eq!(slugify("Don't Stop Me Now"), "dont-stop-me-now");
    }
}
