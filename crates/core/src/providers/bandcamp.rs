//! Bandcamp: tracks whose artist published the lyrics on the track page. Found through
//! Bandcamp's public search endpoint, so no key is needed.

use super::web::{balanced_div, get_html, html_to_text, BROWSER_UA};
use super::CheckStatus;
use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use serde_json::{json, Value};

pub struct Bandcamp;

static LYRICS_DIV: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div class="tralbumData lyricsText">"#).unwrap());
/// A track page, as the web-search fallback finds it.
static PAGE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https://[a-z0-9-]+\.bandcamp\.com/track/[^/?#]+/?$").unwrap());
/// `<meta property="og:title" content="Holocene, by Bon Iver">`
static OG_TITLE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<meta property="og:title" content="([^"]*), by ([^"]*)">"#).unwrap());

/// The lyrics the artist published on a track page.
fn page_lyrics(html: &str) -> Option<String> {
    let m = LYRICS_DIV.find(html)?;
    // the page source is indented and wrapped; only <br> means a line break
    let inner = balanced_div(html, m.end()).replace(['\r', '\n'], " ");
    let text = html_to_text(&inner).lines().map(str::trim).collect::<Vec<_>>().join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// `(title, artist)` from a track page's `og:title`.
fn page_names(html: &str) -> Option<(String, String)> {
    let c = OG_TITLE.captures(html)?;
    Some((html_escape::decode_html_entities(&c[1]).into_owned(), html_escape::decode_html_entities(&c[2]).into_owned()))
}

#[async_trait]
impl Provider for Bandcamp {
    fn id(&self) -> &'static str {
        "bandcamp"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let res = http
            .post("https://bandcamp.com/api/bcsearch_public_api/1/autocomplete_elastic")
            .header("User-Agent", BROWSER_UA)
            .json(&json!({ "search_text": format!("{} {}", q.title, q.artist), "search_filter": "t", "full_page": false, "fan_id": null }))
            .send()
            .await
            .context("bandcamp search")?;
        let res: Value = res.check_status("bandcamp")?.json().await.context("bandcamp search: bad json")?;
        let results = res.pointer("/auto/results").and_then(Value::as_array).cloned().unwrap_or_default();
        let best = results
            .iter()
            .filter(|r| r.get("type").and_then(Value::as_str) == Some("t"))
            .filter_map(|r| {
                let name = r.get("name")?.as_str()?.to_string();
                let band = r.get("band_name")?.as_str()?.to_string();
                let url = r.get("item_url_path")?.as_str()?.to_string();
                let sc = score(q, &band, &name, None);
                sc.accepted(cfg.min_score).then_some((sc.overall, url, band, name))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((sc, url, band, name)) = best else { return Ok(None) };
        if !url.starts_with("https://") {
            return Ok(None);
        }
        let html = get_html(http, &url, &[]).await?;
        Ok(page_lyrics(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "bandcamp".into(),
            url: Some(url),
            matched_artist: band,
            matched_title: name,
            score: sc,
            ..Default::default()
        }))
    }

    fn supports_url(&self, url: &str) -> bool {
        PAGE_URL.is_match(url)
    }

    /// A track page from a web search, used only when its title and artist match the query.
    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let html = get_html(http, url, &[]).await?;
        let Some((title, artist)) = page_names(&html) else { return Ok(None) };
        let sc = score(q, &artist, &title, None);
        if !sc.accepted(cfg.min_score) {
            return Ok(None);
        }
        Ok(page_lyrics(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "bandcamp".into(),
            url: Some(url.to_string()),
            matched_artist: artist,
            matched_title: title,
            score: sc.overall,
            ..Default::default()
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // trimmed from a bandcamp.com track page, 2026-11
    const PAGE: &str = r#"<meta property="og:title" content="Holocene, by Bon Iver">
    <div class="tralbumData lyricsText">Someway, baby, it&#39;s part of me, apart from me<br>
        You&#39;re laying waste to Halloween<br><br>
        And at once I knew</div><div class="other">x</div>"#;

    #[test]
    fn reads_a_track_page() {
        assert_eq!(page_names(PAGE), Some(("Holocene".into(), "Bon Iver".into())));
        assert_eq!(page_lyrics(PAGE).unwrap(), "Someway, baby, it's part of me, apart from me\nYou're laying waste to Halloween\n\nAnd at once I knew");
        assert_eq!(page_lyrics("<html>no lyrics here</html>"), None);
    }

    #[test]
    fn recognises_track_pages_only() {
        assert!(Bandcamp.supports_url("https://boniver.bandcamp.com/track/holocene"));
        assert!(!Bandcamp.supports_url("https://boniver.bandcamp.com/album/bon-iver"));
        assert!(!Bandcamp.supports_url("http://boniver.bandcamp.com/track/holocene"));
        assert!(!Bandcamp.supports_url("https://bandcamp.com.evil.example/track/x"));
    }
}
