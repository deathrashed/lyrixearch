//! LyricsFreak: search page -> best row -> song page.

use super::web::{balanced_div, get_html, html_to_text};
use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct LyricsFreak;

const BASE: &str = "https://www.lyricsfreak.com";

static ROW_ARTIST: Lazy<Regex> = Lazy::new(|| Regex::new(r#"data-sorting-artist="([^"]*)""#).unwrap());
static ROW_SONG: Lazy<Regex> = Lazy::new(|| Regex::new(r#"data-sorting-song="([^"]*)""#).unwrap());
static ROW_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?s)<a[^>]*?href="([^"]+)"[^>]*?class="song""#).unwrap());
static CONTENT: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div[^>]*id=["']content["'][^>]*>"#).unwrap());
static URL_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://(www\.)?lyricsfreak\.com/[^/]+/[^/]+/[^/]+\.html$").unwrap());

fn page_lyrics(html: &str) -> Option<String> {
    let m = CONTENT.find(html)?;
    let text = html_to_text(balanced_div(html, m.end()));
    (!text.trim().is_empty()).then_some(text)
}

#[async_trait]
impl Provider for LyricsFreak {
    fn id(&self) -> &'static str {
        "lyricsfreak"
    }

    fn supports_url(&self, url: &str) -> bool {
        URL_RE.is_match(url)
    }

    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let html = get_html(http, url, &[("Referer", "https://www.lyricsfreak.com/search.php")]).await?;
        // <title>Artist - Title Lyrics | LyricsFreak</title>
        let title_tag = regex::Regex::new(r"(?is)<title>(.*?)</title>").unwrap();
        let t = title_tag.captures(&html).map(|c| html_escape::decode_html_entities(&c[1]).into_owned()).unwrap_or_default();
        let t = t.split('|').next().unwrap_or("").trim().trim_end_matches("Lyrics").trim().to_string();
        let (artist, title) = super::web::split_artist_title(&t).unwrap_or_default();
        let sc = score(q, &artist, &title, None);
        if !sc.accepted(cfg.min_score) {
            return Ok(None);
        }
        Ok(page_lyrics(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "lyricsfreak".into(),
            url: Some(url.to_string()),
            matched_artist: artist,
            matched_title: title,
            score: sc.overall,
            ..Default::default()
        }))
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let url = format!("{BASE}/search.php?q={}&type=song", urlencoding::encode(&format!("{} {}", q.artist, q.title)));
        let html = get_html(http, &url, &[("Referer", "https://www.lyricsfreak.com/search.php")]).await?;

        // Rows are introduced by their data-sorting-artist attribute; split on it.
        let mut best: Option<(f32, String)> = None;
        for chunk in html.split("data-sorting-artist=").skip(1) {
            let chunk = format!("data-sorting-artist={chunk}");
            let (Some(artist), Some(song)) = (
                ROW_ARTIST.captures(&chunk).map(|c| html_escape::decode_html_entities(&c[1]).into_owned()),
                ROW_SONG.captures(&chunk).map(|c| html_escape::decode_html_entities(&c[1]).into_owned()),
            ) else {
                continue;
            };
            let Some(href) = ROW_LINK.captures(&chunk).map(|c| c[1].to_string()) else { continue };
            let sc = score(q, &artist, &song, None);
            if sc.accepted(cfg.min_score) && best.as_ref().is_none_or(|(b, _)| sc.overall > *b) {
                let full = if href.starts_with("http") { href } else { format!("{BASE}{href}") };
                best = Some((sc.overall, full));
            }
        }
        match best {
            Some((_, page)) => self.fetch_url(http, &page, q, cfg).await,
            None => Ok(None),
        }
    }
}
