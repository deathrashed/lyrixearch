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
        let html = get_html(http, &url, &[]).await?;
        let mut best: Option<(f32, String, String, String)> = None;
        for c in ROW.captures_iter(&html) {
            let title = html_escape::decode_html_entities(PAREN_TAIL.replace(c[2].trim(), "").trim()).into_owned();
            let artist = html_escape::decode_html_entities(c[3].trim()).into_owned();
            let sc = score(q, &artist, &title, None);
            if sc.accepted(cfg.min_score) && best.as_ref().is_none_or(|b| sc.overall > b.0) {
                best = Some((sc.overall, format!("{BASE}{}", &c[1]), artist, title));
            }
        }
        let Some((sc, page, artist, title)) = best else { return Ok(None) };
        let song = get_html(http, &page, &[]).await?;
        let Some(m) = BODY.find(&song) else { return Ok(None) };
        let inner = FURIGANA.replace_all(balanced_div(&song, m.end()), "");
        let text = html_to_text(&inner);
        Ok((!text.trim().is_empty()).then(|| Lyrics {
            plain: Some(text),
            source: "utaten".into(),
            url: Some(page),
            matched_artist: artist,
            matched_title: title,
            score: sc,
            ..Default::default()
        }))
    }
}
