//! AZLyrics: large English catalogue of plain lyrics.
//!
//! The search form carries a token from `geo.js`; results link to `/lyrics/<artist>/<song>.html`,
//! whose lyric is the one `<div>` without attributes. The site shows a captcha to clients it does
//! not trust; that comes back as a [`super::Challenge`], and the site is then read through the app's
//! browser where the person can answer it.

use super::web::{get_html, html_to_text};
use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct AzLyrics;

const BASE: &str = "https://www.azlyrics.com";

static PAGE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://(www\.)?azlyrics\.com/lyrics/[^/]+/[^/]+\.html$").unwrap());
static ATTR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"setAttribute\(\s*"(.+?)"\s*,\s*"(.+?)"\s*\)"#).unwrap());
static ROW: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?s)<tr>.*?<td[^>]*>.*?<a[^>]*href="([^"]+)"[^>]*>.*?<b>(.*?)</b>.*?<b>([^<]*)</b>.*?</td>\s*</tr>"#).unwrap());
static BARE_DIV: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<div>(.*?)</div>").unwrap());
static COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static ARTIST_VAR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"ArtistName\s*=\s*(['"])(.+?)['"]\s*;"#).unwrap());
static SONG_VAR: Lazy<Regex> = Lazy::new(|| Regex::new(r#"SongName\s*=\s*(['"])(.+?)['"]\s*;"#).unwrap());
static TAIL: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?is)\n\s*(writer|songwriter)\s*\(?s?\)?\s*:.*$|\n\s*submit corrections.*$").unwrap());

/// The hidden search field from `geo.js`: `ep.setAttribute("name", "x"); ep.setAttribute("value", "…")`.
fn search_token(js: &str) -> Option<(String, String)> {
    let mut name = None;
    let mut value = None;
    for c in ATTR.captures_iter(js) {
        match &c[1] {
            "name" => name = Some(c[2].to_string()),
            "value" => value = Some(c[2].to_string()),
            _ => {}
        }
    }
    Some((name?, value?))
}

/// Search rows as `(page url, title, artist)`; titles come quoted.
fn search_rows(html: &str) -> Vec<(String, String, String)> {
    ROW.captures_iter(html)
        .filter(|c| PAGE_URL.is_match(&c[1]))
        .map(|c| {
            let title = html_escape::decode_html_entities(c[2].trim().trim_matches('"')).into_owned();
            (c[1].to_string(), title, html_escape::decode_html_entities(c[3].trim()).into_owned())
        })
        .collect()
}

fn song_text(html: &str) -> Option<String> {
    let inner = BARE_DIV.captures(html)?;
    let text = html_to_text(&COMMENT.replace_all(&inner[1], ""));
    let text = TAIL.replace(&text, "").trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn page_names(html: &str) -> Option<(String, String)> {
    let artist = ARTIST_VAR.captures(html)?[2].to_string();
    let title = SONG_VAR.captures(html)?[2].to_string();
    Some((html_escape::decode_html_entities(&title).into_owned(), html_escape::decode_html_entities(&artist).into_owned()))
}

#[async_trait]
impl Provider for AzLyrics {
    fn id(&self) -> &'static str {
        "azlyrics"
    }

    fn supports_url(&self, url: &str) -> bool {
        PAGE_URL.is_match(url)
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let terms = urlencoding::encode(&format!("{} {}", q.artist, q.title)).into_owned();
        let token = get_html(http, &format!("{BASE}/geo.js"), &[("Referer", "https://www.azlyrics.com/")]).await.ok().and_then(|js| search_token(&js));
        let extra = token.map(|(k, v)| format!("&{k}={}", urlencoding::encode(&v))).unwrap_or_default();
        let mut rows = search_rows(&get_html(http, &format!("{BASE}/search/?q={terms}{extra}"), &[("Referer", "https://www.azlyrics.com/")]).await?);
        if rows.is_empty() {
            // the older search host
            let alt = format!("https://search.azlyrics.com/search.php?q={terms}{extra}");
            rows = search_rows(&get_html(http, &alt, &[("Referer", "https://www.azlyrics.com/")]).await.unwrap_or_default());
        }
        let best = rows
            .into_iter()
            .map(|(page, title, artist)| (score(q, &artist, &title, None), page))
            .filter(|(sc, _)| sc.accepted(cfg.min_score))
            .max_by(|a, b| a.0.overall.total_cmp(&b.0.overall));
        match best {
            Some((_, page)) => self.fetch_url(http, &page, q, cfg).await,
            None => Ok(None),
        }
    }

    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let html = get_html(http, url, &[("Referer", "https://www.azlyrics.com/")]).await?;
        let Some((title, artist)) = page_names(&html) else { return Ok(None) };
        let sc = score(q, &artist, &title, None);
        if !sc.accepted(cfg.min_score) {
            return Ok(None);
        }
        Ok(song_text(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "azlyrics".into(),
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

    #[test]
    fn reads_the_search_token() {
        let js =
            r#"var ep = document.createElement("input"); ep.setAttribute("type", "hidden"); ep.setAttribute("name", "x"); ep.setAttribute("value", "abc123");"#;
        assert_eq!(search_token(js), Some(("x".into(), "abc123".into())));
    }

    #[test]
    fn reads_search_rows() {
        let html = r#"<table><tr><td class="text-left visitedlyr">1. <a href="https://www.azlyrics.com/lyrics/queen/bohemianrhapsody.html"><span><b>"Bohemian Rhapsody"</b></span></a>&nbsp;-&nbsp;<b>Queen</b><br></td></tr>
        <tr><td class="text-left"><a href="https://www.azlyrics.com/q/queen.html"><b>Queen</b> <b>x</b></a></td></tr></table>"#;
        assert_eq!(search_rows(html), vec![("https://www.azlyrics.com/lyrics/queen/bohemianrhapsody.html".into(), "Bohemian Rhapsody".into(), "Queen".into())]);
    }

    #[test]
    fn reads_a_song_page() {
        let html = r#"<script>ArtistName = "Queen"; SongName = "Bohemian Rhapsody";</script>
        <div class="ringtone"></div><b>"Bohemian Rhapsody"</b><br><br>
        <div>
<!-- Usage of azlyrics.com content by any third-party lyrics provider is prohibited by our licensing agreement. Sorry about that. -->
Is this the real life?<br>
Is this just fantasy?<br>
<br>
Caught in a landslide
</div><br><br><div class="smt">Writer(s): Freddie Mercury</div>"#;
        assert_eq!(page_names(html), Some(("Bohemian Rhapsody".into(), "Queen".into())));
        assert_eq!(song_text(html).unwrap(), "Is this the real life?\nIs this just fantasy?\n\nCaught in a landslide");
    }

    #[test]
    fn recognises_song_pages() {
        assert!(AzLyrics.supports_url("https://www.azlyrics.com/lyrics/queen/bohemianrhapsody.html"));
        assert!(!AzLyrics.supports_url("https://www.azlyrics.com/q/queen.html"));
    }
}
