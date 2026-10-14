//! Bandcamp: tracks whose artist published the lyrics on the track page. Found through
//! Bandcamp's public search endpoint, so no key is needed.

use super::web::{balanced_div, get_html, html_to_text, BROWSER_UA};
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
        if res.status().as_u16() == 429 {
            return Err(super::RateLimited::from_response(&res, "bandcamp rate limited (HTTP 429)").into());
        }
        let res: Value = res.error_for_status()?.json().await.context("bandcamp search: bad json")?;
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
        let Some(m) = LYRICS_DIV.find(&html) else { return Ok(None) };
        // the page source is indented and wrapped; only <br> means a line break
        let inner = balanced_div(&html, m.end()).replace(['\r', '\n'], " ");
        let text = html_to_text(&inner).lines().map(str::trim).collect::<Vec<_>>().join("\n");
        Ok((!text.trim().is_empty()).then(|| Lyrics {
            plain: Some(text),
            source: "bandcamp".into(),
            url: Some(url),
            matched_artist: band,
            matched_title: name,
            score: sc,
            ..Default::default()
        }))
    }
}
