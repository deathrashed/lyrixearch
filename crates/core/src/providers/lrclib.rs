//! lrclib.net — open lyrics database with plain and synced (LRC) lyrics.
//! API: <https://lrclib.net/docs>

use super::{Provider, USER_AGENT};
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::Deserialize;

pub struct Lrclib;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Record {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    track_name: String,
    #[serde(default)]
    artist_name: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    instrumental: bool,
    #[serde(default)]
    plain_lyrics: Option<String>,
    #[serde(default)]
    synced_lyrics: Option<String>,
}

impl Record {
    fn into_lyrics(self, s: f32) -> Option<Lyrics> {
        let plain = self.plain_lyrics.filter(|t| !t.trim().is_empty());
        let synced = self.synced_lyrics.filter(|t| !t.trim().is_empty());
        if plain.is_none() && synced.is_none() {
            return None;
        }
        Some(Lyrics {
            plain,
            synced,
            source: "lrclib".into(),
            url: self.id.map(|id| format!("https://lrclib.net/api/get/{id}")),
            matched_artist: self.artist_name,
            matched_title: self.track_name,
            score: s,
        })
    }
}

#[async_trait]
impl Provider for Lrclib {
    fn id(&self) -> &'static str {
        "lrclib"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        // 1) exact lookup — cheap and precise when tags are good
        let mut req = http
            .get("https://lrclib.net/api/get")
            .header("User-Agent", USER_AGENT)
            .header("Lrclib-Client", USER_AGENT)
            .query(&[("track_name", q.title.as_str()), ("artist_name", q.artist.as_str())]);
        if let Some(album) = &q.album {
            req = req.query(&[("album_name", album.as_str())]);
        }
        if let Some(d) = q.duration_secs {
            req = req.query(&[("duration", d.to_string())]);
        }
        let resp = req.send().await.context("lrclib get")?;
        if resp.status().is_success() {
            let rec: Record = resp.json().await.context("lrclib get: bad json")?;
            if !rec.instrumental {
                let s = score(q, &rec.artist_name, &rec.track_name, rec.duration);
                if s.accepted(cfg.min_score) {
                    if let Some(l) = rec.into_lyrics(s.overall) {
                        return Ok(Some(l));
                    }
                }
            }
        } else if resp.status().as_u16() == 429 {
            return Err(super::RateLimited::from_response(&resp, "lrclib rate limited (HTTP 429)").into());
        } else if resp.status().is_client_error() && resp.status().as_u16() != 404 {
            anyhow::bail!("lrclib get: HTTP {}", resp.status());
        }
        // 404 means no exact match, and a 5xx is the server having a bad moment: either way the
        // fuzzy search below may still answer.

        // 2) fuzzy search, pick the best-scoring candidate that has lyrics
        let resp = http
            .get("https://lrclib.net/api/search")
            .header("User-Agent", USER_AGENT)
            .header("Lrclib-Client", USER_AGENT)
            .query(&[("q", format!("{} {}", q.artist, q.title))])
            .send()
            .await
            .context("lrclib search")?;
        if resp.status().as_u16() == 429 || resp.status().is_server_error() {
            // the service is busy or having a bad moment: leave it alone for a while instead of retrying every track
            return Err(super::RateLimited::from_response(&resp, &format!("lrclib busy (HTTP {})", resp.status().as_u16())).into());
        }
        if !resp.status().is_success() {
            anyhow::bail!("lrclib search: HTTP {}", resp.status());
        }
        let recs: Vec<Record> = resp.json().await.context("lrclib search: bad json")?;
        let best = recs
            .into_iter()
            .filter(|r| !r.instrumental && (r.plain_lyrics.is_some() || r.synced_lyrics.is_some()))
            .map(|r| {
                let s = score(q, &r.artist_name, &r.track_name, r.duration);
                (s, r)
            })
            .filter(|(s, _)| s.accepted(cfg.min_score))
            .max_by(|a, b| {
                // prefer synced results when scores are close
                let key = |(s, r): &(crate::matching::MatchScore, Record)| s.overall + if r.synced_lyrics.is_some() { 0.02 } else { 0.0 };
                key(a).total_cmp(&key(b))
            });
        Ok(best.and_then(|(s, r)| r.into_lyrics(s.overall)))
    }
}
