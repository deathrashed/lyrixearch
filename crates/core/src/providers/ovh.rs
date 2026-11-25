//! lyrics.ovh — simple exact-name API, plain lyrics only. Used as a last resort since the
//! service does no fuzzy matching (so the score is 1.0 only by construction).

use super::CheckStatus;
use super::{Provider, USER_AGENT};
use crate::clean::{clean_title, strip_featuring};
use crate::config::Config;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

pub struct LyricsOvh;

#[async_trait]
impl Provider for LyricsOvh {
    fn id(&self) -> &'static str {
        "lyrics_ovh"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, _cfg: &Config) -> Result<Option<Lyrics>> {
        let artist = strip_featuring(&q.artist);
        let title = clean_title(&strip_featuring(&q.title));
        let mut url = reqwest::Url::parse("https://api.lyrics.ovh/v1/")?;
        url.path_segments_mut().map_err(|_| anyhow::anyhow!("bad base url"))?.pop_if_empty().push(&artist).push(&title);

        let resp = http.get(url.clone()).header("User-Agent", USER_AGENT).send().await.context("lyrics.ovh")?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        let json: Value = resp.check_status("lyrics.ovh")?.json().await.context("lyrics.ovh: bad json")?;
        let text = json.get("lyrics").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Ok(None);
        }
        Ok(Some(Lyrics {
            plain: Some(text),
            synced: None,
            source: "lyrics_ovh".into(),
            url: Some(url.to_string()),
            matched_artist: artist,
            matched_title: title,
            score: 0.8,
        }))
    }
}
