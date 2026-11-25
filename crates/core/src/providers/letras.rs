//! Letras.com (and letras.mus.br): large catalogue, strong for Portuguese and Spanish music.
//!
//! Search goes through the site's suggestion index (`solr.sscdn.co/letras/m1`, JSONP). Song pages
//! sit behind Akamai bot protection that refuses plain HTTP clients, so they are loaded in the app's
//! browser (see [`super::web::route_through_browser`]); without one (the CLI) the source reports it.

use super::web::{balanced_div, get_html, get_text, html_to_text};
use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;

pub struct Letras;

pub const BASE: &str = "https://www.letras.com";
const SEARCH: &str = "https://solr.sscdn.co/letras/m1/";

static PAGE_URL: Lazy<Regex> = Lazy::new(|| Regex::new(r"^https?://(www\.)?letras\.(com|mus\.br)/[^/]+/[^/]+/?$").unwrap());
static LYRIC: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div[^>]*class="[^"]*\blyric-original\b[^"]*"[^>]*>"#).unwrap());
static TITLE_BLOCK: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<div[^>]*class="[^"]*\btitle-content\b[^"]*"[^>]*>"#).unwrap());
static HEADING: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<(h\d)[^>]*>(.*?)</h\d\s*>").unwrap());
static TAG: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<[^>]*>").unwrap());

#[derive(serde::Deserialize)]
struct Doc {
    #[serde(default)]
    art: String,
    #[serde(default)]
    txt: String,
    #[serde(default)]
    dns: String,
    #[serde(default)]
    url: String,
    /// "2" is a song ("1" an artist)
    #[serde(default)]
    t: String,
}

/// Songs from a suggestion response (`LetrasSug({...})`) as `(page url, artist, title)`.
fn search_rows(body: &str) -> Vec<(String, String, String)> {
    let json = body.trim().trim_start_matches("LetrasSug(").trim_end_matches(';').trim_end_matches(')');
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let docs: Vec<Doc> = serde_json::from_value(v["response"]["docs"].clone()).unwrap_or_default();
    docs.into_iter().filter(|d| d.t == "2" && !d.dns.is_empty() && !d.url.is_empty()).map(|d| (format!("{BASE}/{}/{}/", d.dns, d.url), d.art, d.txt)).collect()
}

fn plain(fragment: &str) -> String {
    html_escape::decode_html_entities(TAG.replace_all(fragment, "").trim()).into_owned()
}

/// `(title, artist)`: the page's title block has the song as its first heading, the artist second.
fn page_names(html: &str) -> Option<(String, String)> {
    let m = TITLE_BLOCK.find(html)?;
    let block = balanced_div(html, m.end());
    let mut heads = HEADING.captures_iter(block).map(|c| plain(&c[2])).filter(|s| !s.is_empty());
    Some((heads.next()?, heads.next()?))
}

fn song_text(html: &str) -> Option<String> {
    let m = LYRIC.find(html)?;
    let text = html_to_text(balanced_div(html, m.end()));
    (!text.trim().is_empty()).then_some(text)
}

#[async_trait]
impl Provider for Letras {
    fn id(&self) -> &'static str {
        "letras"
    }

    fn supports_url(&self, url: &str) -> bool {
        PAGE_URL.is_match(url)
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let url = format!("{SEARCH}?q={}&wt=json&callback=LetrasSug", urlencoding::encode(&format!("{} {}", q.artist, q.title)));
        let body = get_text(http, &url, &[("Referer", "https://www.letras.com/")]).await?;
        let best = search_rows(&body)
            .into_iter()
            .map(|(page, artist, title)| (score(q, &artist, &title, None), page))
            .filter(|(sc, _)| sc.accepted(cfg.min_score))
            .max_by(|a, b| a.0.overall.total_cmp(&b.0.overall));
        match best {
            Some((_, page)) => self.fetch_url(http, &page, q, cfg).await,
            None => Ok(None),
        }
    }

    async fn fetch_url(&self, http: &reqwest::Client, url: &str, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let html = get_html(http, url, &[("Referer", "https://www.letras.com/")]).await?;
        let Some((title, artist)) = page_names(&html) else { return Ok(None) };
        let sc = score(q, &artist, &title, None);
        if !sc.accepted(cfg.min_score) {
            return Ok(None);
        }
        Ok(song_text(&html).map(|text| Lyrics {
            plain: Some(text),
            source: "letras".into(),
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

    const SUG: &str = r#"LetrasSug({"response":{"numFound":2,"docs":[
        {"art":"Legião Urbana","dns":"legiao-urbana","txt":"Legião Urbana","t":"1","url":""},
        {"art":"Legião Urbana","dns":"legiao-urbana","txt":"Tempo Perdido","t":"2","url":"22489"}]}})"#;

    const PAGE: &str = r#"<div class="title-content"><h1 class="textStyle-primary">Tempo Perdido</h1>
        <h2 class="textStyle-secondary"><a href="/legiao-urbana/">Legião Urbana</a></h2></div>
        <div class="lyric-original"><p>Todos os dias quando acordo<br/>Não tenho mais</p><p>O tempo que passou</p></div><div class="x">y</div>"#;

    #[test]
    fn reads_suggestions_songs_only() {
        let rows = search_rows(SUG);
        assert_eq!(rows, vec![("https://www.letras.com/legiao-urbana/22489/".into(), "Legião Urbana".into(), "Tempo Perdido".into())]);
    }

    #[test]
    fn reads_a_song_page() {
        assert_eq!(page_names(PAGE), Some(("Tempo Perdido".into(), "Legião Urbana".into())));
        assert_eq!(song_text(PAGE).unwrap(), "Todos os dias quando acordo\nNão tenho mais\n\nO tempo que passou");
    }

    #[test]
    fn recognises_song_pages() {
        assert!(Letras.supports_url("https://www.letras.com/legiao-urbana/22489/"));
        assert!(Letras.supports_url("https://www.letras.mus.br/legiao-urbana/22489/"));
        assert!(!Letras.supports_url("https://www.letras.com/legiao-urbana/"));
    }
}
