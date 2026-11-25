//! UtaTen: Japanese lyrics. Only asked when the artist or title contains Japanese text.

use super::web::{balanced_div, get_html, html_to_text};
use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct UtaTen;

const BASE: &str = "https://utaten.com";

static ROW: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"(?s)<p class="searchResult__title">\s*<a href="(/lyric/[^"]+)">\s*(.*?)\s*</a>.*?class="searchResult__artist">\s*<p>\s*<a href="/artist/\d+/">\s*(.*?)\s*</a>"#).unwrap()
});
static BODY: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div class="hiragana"[^>]*>"#).unwrap());
static FURIGANA: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?s)<span class="rt">.*?</span>"#).unwrap());
static PAREN_TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"[\(（][^\)）]*[\)）]\s*$").unwrap());
/// A song page, as the web-search fallback finds it.
static PAGE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https://utaten\.com/lyric/[a-z0-9]+/?$").unwrap());
/// `<title>夜に駆ける 歌詞 YOASOBI ふりがな付 - うたてん</title>`: title, then artist.
static PAGE_TITLE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<title>\s*(.*?)\s+歌詞\s+(.*?)\s+ふりがな付").unwrap());

/// Search result rows as `(page path, title, artist)`.
fn search_rows(html: &str) -> Vec<(String, String, String)> {
    ROW.captures_iter(html)
        .map(|c| {
            let title = html_escape::decode_html_entities(PAREN_TAIL.replace(c[2].trim(), "").trim()).into_owned();
            let artist = html_escape::decode_html_entities(c[3].trim()).into_owned();
            (c[1].to_string(), title, artist)
        })
        .collect()
}

/// The lyric on a song page, furigana removed.
fn song_text(html: &str) -> Option<String> {
    let m = BODY.find(html)?;
    let inner = FURIGANA.replace_all(balanced_div(html, m.end()), "");
    let text = html_to_text(&inner);
    (!text.trim().is_empty()).then_some(text)
}

/// `(title, artist)` from a song page's `<title>`.
fn page_names(html: &str) -> Option<(String, String)> {
    let c = PAGE_TITLE.captures(html)?;
    Some((html_escape::decode_html_entities(c[1].trim()).into_owned(), html_escape::decode_html_entities(c[2].trim()).into_owned()))
}

pub(super) fn has_japanese(s: &str) -> bool {
    s.chars().any(|c| matches!(c, '\u{3040}'..='\u{30ff}' | '\u{4e00}'..='\u{9fff}'))
}

#[async_trait]
impl Provider for UtaTen {
    fn id(&self) -> &'static str {
        "utaten"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        if !has_japanese(&q.artist) && !has_japanese(&q.title) {
            return Ok(None);
        }
        let url = format!("{BASE}/search?sort=popular_sort_asc&artist_name={}&title={}", urlencoding::encode(&q.artist), urlencoding::encode(&q.title));
        let referer = [("Referer", "https://utaten.com/")];
        let mut html = get_html(http, &url, &referer).await?;
        if html.trim().is_empty() {
            // UtaTen answers a search with an empty page until the visitor has its session cookie
            // (seen 2026-11); the engine's client keeps cookies, so one visit to the home page is enough.
            get_html(http, &format!("{BASE}/"), &[]).await?;
            html = get_html(http, &url, &referer).await?;
        }
        let mut best: Option<(f32, String, String, String)> = None;
        for (path, title, artist) in search_rows(&html) {
            let sc = score(q, &artist, &title, None);
            if sc.accepted(cfg.min_score) && best.as_ref().is_none_or(|b| sc.overall > b.0) {
                best = Some((sc.overall, format!("{BASE}{path}"), artist, title));
            }
        }
        let Some((sc, page, artist, title)) = best else { return Ok(None) };
        let song = get_html(http, &page, &referer).await?;
        Ok(song_text(&song).map(|text| Lyrics {
            plain: Some(text),
            source: "utaten".into(),
            url: Some(page),
            matched_artist: artist,
            matched_title: title,
            score: sc,
            ..Default::default()
        }))
    }

    fn supports_url(&self, url: &str) -> bool {
        PAGE_URL.is_match(url)
    }

    /// A song page from a web search, used only when its title and artist match the query.
    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let html = get_html(http, url, &[("Referer", "https://utaten.com/")]).await?;
        let Some((title, artist)) = page_names(&html) else { return Ok(None) };
        let sc = score(q, &artist, &title, None);
        if !sc.accepted(cfg.min_score) {
            return Ok(None);
        }
        Ok(song_text(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "utaten".into(),
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

    // trimmed from utaten.com, 2026-11
    const SEARCH: &str = r#"<tr><td><p class="searchResult__title">
          <a href="/lyric/rq20031716/">
          夜に駆ける        </a><span class="popularIcon">人気</span></p></td>
    <td class="searchResult__artist"><p>
                  <a href="/artist/37708/">
                  YOASOBI        </a></p></td></tr>
    <tr><td><p class="searchResult__title"><a href="/lyric/mi21063001/">Into The Night (夜に駆ける 英語ver.)</a></p></td>
    <td class="searchResult__artist"><p><a href="/artist/37708/">YOASOBI</a></p></td></tr>"#;

    const SONG: &str = r#"<title>夜に駆ける 歌詞 YOASOBI ふりがな付 - うたてん</title>
    <div class="hiragana">沈<span class="rt">しず</span>むように<br />溶<span class="rt">と</span>けてゆくように<br /><br />二人<span class="rt">ふたり</span>だけの空<span class="rt">そら</span>が</div><div class="other">x</div>"#;

    #[test]
    fn reads_search_rows() {
        let rows = search_rows(SEARCH);
        assert_eq!(rows[0], ("/lyric/rq20031716/".into(), "夜に駆ける".into(), "YOASOBI".into()));
        assert_eq!(rows[1].1, "Into The Night", "a trailing bracket is dropped from the title");
    }

    #[test]
    fn reads_a_song_page_without_furigana() {
        assert_eq!(song_text(SONG).unwrap(), "沈むように\n溶けてゆくように\n\n二人だけの空が");
        assert_eq!(page_names(SONG), Some(("夜に駆ける".into(), "YOASOBI".into())));
    }

    #[test]
    fn recognises_song_pages_only() {
        assert!(UtaTen.supports_url("https://utaten.com/lyric/rq20031716/"));
        assert!(!UtaTen.supports_url("https://utaten.com/artist/37708/"));
        assert!(!UtaTen.supports_url("http://utaten.com.evil.example/lyric/x/"));
    }
}
