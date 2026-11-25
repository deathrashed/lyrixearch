//! DarkLyrics: metal lyrics, one page per album with every track on it. The site only answers
//! over plain http, so that is what is used.
//!
//! The original app sent a computed `lastvisitts` cookie. Checked on 2026-11-25: album pages
//! answer in full without it, so none is sent.

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
/// An album page, as the web-search fallback finds it.
static PAGE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://(?:www\.)?darklyrics\.com/lyrics/([a-z0-9]+)/([a-z0-9]+)\.html(?:#\d+)?$").unwrap());

/// "Mötley Crüe" -> "motleycrue": the site keeps only ASCII letters and digits, accents folded.
fn slug(s: &str) -> String {
    super::web::ascii_fold(&clean_title(s)).to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
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
        // the page source wraps after every <br />; only the <br /> itself is a line break
        let text = html_to_text(&body[range.end..stop].replace(['\r', '\n'], " ")).lines().map(str::trim).collect::<Vec<_>>().join("\n");
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

    fn supports_url(&self, url: &str) -> bool {
        PAGE_URL.is_match(url)
    }

    /// An album page from a web search: only used when its artist is the one asked for, and then
    /// the best-matching track on it is taken.
    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let Some(c) = PAGE_URL.captures(url) else { return Ok(None) };
        if !artist_slugs(&q.artist).iter().any(|a| a == &c[1]) {
            return Ok(None);
        }
        let page = url.split('#').next().unwrap_or(url);
        let html = get_html(http, page, &[]).await?;
        Ok(best_track(&html, q, cfg.min_score).map(|(sc, num, title, text)| Lyrics {
            plain: Some(text),
            source: "darklyrics".into(),
            url: Some(format!("{page}#{num}")),
            matched_artist: q.artist.clone(),
            matched_title: title,
            score: sc,
            ..Default::default()
        }))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_newlines_do_not_double_the_lines() {
        let html = "<div class=\"lyrics\">\n<h3><a name=\"1\">1. Battery</a></h3><br />\nLashing out<br />\nWeak are ripped<br />\n<br />\nSmashing through<br />\n<div class=\"thanks\">x</div>";
        let q = Query { artist: "Metallica".into(), title: "Battery".into(), ..Default::default() };
        let (_, _, _, text) = best_track(html, &q, 0.8).unwrap();
        assert_eq!(text, "Lashing out\nWeak are ripped\n\nSmashing through");
    }

    #[test]
    fn slugs_fold_accents_and_drop_the() {
        assert_eq!(slug("Mötley Crüe"), "motleycrue");
        assert_eq!(slug("Iron Maiden"), "ironmaiden");
        assert_eq!(artist_slugs("The Black Dahlia Murder"), vec!["theblackdahliamurder".to_string(), "blackdahliamurder".to_string()]);
    }

    #[test]
    fn recognises_album_pages_only() {
        assert!(DarkLyrics.supports_url("http://www.darklyrics.com/lyrics/ironmaiden/pieceofmind.html#1"));
        assert!(DarkLyrics.supports_url("https://darklyrics.com/lyrics/metallica/masterofpuppets.html"));
        assert!(!DarkLyrics.supports_url("http://www.darklyrics.com/m/metallica.html"));
        assert!(!DarkLyrics.supports_url("https://genius.com/Metallica-battery-lyrics"));
    }

    #[test]
    fn picks_the_matching_track_and_rejects_others() {
        let html = "<div class=\"lyrics\"><h3><a name=\"1\">1. Battery</a></h3><br />Lashing out<br />\n<h3><a name=\"2\">2. Master Of Puppets</a></h3><br />End of passion play<br />Crumbling away<br />\n<div class=\"thanks\">x</div>";
        let q = Query { artist: "Metallica".into(), title: "Master of Puppets".into(), ..Default::default() };
        let (_, num, title, text) = best_track(html, &q, 0.8).unwrap();
        assert_eq!((num.as_str(), title.as_str(), text.as_str()), ("2", "Master Of Puppets", "End of passion play\nCrumbling away"));
        let q = Query { artist: "Metallica".into(), title: "One".into(), ..Default::default() };
        assert!(best_track(html, &q, 0.8).is_none());
    }
}
