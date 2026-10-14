//! DarkLyrics: metal lyrics, one page per album with every track on it. The site only answers
//! over plain http, so that is what is used.

use super::web::{get_html, html_to_text};
use super::Provider;
use crate::clean::clean_title;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct DarkLyrics;

const BASE: &str = "http://www.darklyrics.com";

static TRACK_HEAD: Lazy<Regex> = Lazy::new(|| Regex::new(r#"(?s)<h3><a name="(\d+)">(?:\d+\.\s*)?(.*?)</a></h3>"#).unwrap());
static ALBUM_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r#"lyrics/([a-z0-9]+)/([a-z0-9]+)\.html"#).unwrap());

fn slug(s: &str) -> String {
    clean_title(s).to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

fn artist_slugs(artist: &str) -> Vec<String> {
    let full = slug(artist);
    let mut v = vec![full.clone()];
    if let Some(rest) = artist.to_lowercase().strip_prefix("the ") {
        v.push(slug(rest));
    }
    v.retain(|s| !s.is_empty());
    v.dedup();
    v
}

/// Best-matching track on an album page, as `(score, number, title, text)`.
fn best_track(html: &str, q: &Query, min: f32) -> Option<(f32, String, String, String)> {
    let start = html.find(r#"<div class="lyrics">"#)?;
    let body = &html[start..];
    let end = body.find(r#"<div class="thanks">"#).or_else(|| body.find(r#"<div class="note">"#)).unwrap_or(body.len());
    let body = &body[..end];
    let heads: Vec<_> =
        TRACK_HEAD.captures_iter(body).map(|c| (c.get(0).unwrap().range(), c[1].to_string(), html_escape::decode_html_entities(&c[2]).into_owned())).collect();
    let mut best: Option<(f32, String, String, String)> = None;
    for (i, (range, num, title)) in heads.iter().enumerate() {
        let sc = score(q, &q.artist, title, None);
        if !sc.accepted(min) || best.as_ref().is_some_and(|b| b.0 >= sc.overall) {
            continue;
        }
        let stop = heads.get(i + 1).map(|h| h.0.start).unwrap_or(body.len());
        let text = html_to_text(&body[range.end..stop]);
        if !text.trim().is_empty() {
            best = Some((sc.overall, num.clone(), title.clone(), text));
        }
    }
    best
}

#[async_trait]
impl Provider for DarkLyrics {
    fn id(&self) -> &'static str {
        "darklyrics"
    }

    /// Up to a handful of album pages from a slow server.
    fn timeout(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs(45))
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        for artist in artist_slugs(&q.artist) {
            // the album named in the tags first, then whatever albums the artist page lists
            let mut albums: Vec<String> = q.album.as_deref().map(slug).filter(|s| !s.is_empty()).into_iter().collect();
            let mut listed = false;
            let mut tried = 0;
            loop {
                for album in albums.drain(..) {
                    if tried >= 6 {
                        break;
                    }
                    tried += 1;
                    let url = format!("{BASE}/lyrics/{artist}/{album}.html");
                    let html = get_html(http, &url, &[]).await?;
                    if let Some((sc, num, title, text)) = best_track(&html, q, cfg.min_score) {
                        return Ok(Some(Lyrics {
                            plain: Some(text),
                            source: "darklyrics".into(),
                            url: Some(format!("{url}#{num}")),
                            matched_artist: q.artist.clone(),
                            matched_title: title,
                            score: sc,
                            ..Default::default()
                        }));
                    }
                }
                if listed || tried >= 6 {
                    break;
                }
                listed = true;
                let first = artist.chars().next().filter(char::is_ascii_alphabetic).map(|c| c.to_string()).unwrap_or_else(|| "19".into());
                let index = get_html(http, &format!("{BASE}/{first}/{artist}.html"), &[]).await?;
                let tried_first = q.album.as_deref().map(slug).unwrap_or_default();
                for c in ALBUM_LINK.captures_iter(&index) {
                    if c[1] == artist && c[2] != tried_first && !albums.contains(&c[2].to_string()) {
                        albums.push(c[2].to_string());
                    }
                }
                if albums.is_empty() {
                    break;
                }
            }
        }
        Ok(None)
    }
}
