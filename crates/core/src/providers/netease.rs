//! NetEase Cloud Music's public web endpoints (unofficial, no key). Good coverage of
//! Asian catalogues, returns synced lyrics.

use super::{Provider, USER_AGENT};
use crate::config::Config;
use crate::lrc;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

pub struct Netease;

#[async_trait]
impl Provider for Netease {
    fn id(&self) -> &'static str {
        "netease"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let search: Value = http
            .get("https://music.163.com/api/cloudsearch/pc")
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://music.163.com/")
            .header("Cookie", "os=pc")
            .query(&[("s", format!("{} {}", q.artist, q.title)), ("type", "1".into()), ("limit", "10".into())])
            .send()
            .await
            .context("netease search")?
            .error_for_status()?
            .json()
            .await
            .context("netease search: bad json")?;

        if let Some(code) = search.get("code").and_then(Value::as_i64) {
            if code != 200 {
                anyhow::bail!("netease api code {code}");
            }
        }
        let songs = search.pointer("/result/songs").and_then(Value::as_array).cloned().unwrap_or_default();

        let best = songs
            .iter()
            .filter_map(|s| {
                let id = s.get("id")?.as_u64()?;
                let name = s.get("name")?.as_str()?.to_string();
                let artists = s
                    .get("ar")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(|x| x.get("name").and_then(Value::as_str)).collect::<Vec<_>>().join(" "))
                    .unwrap_or_default();
                let dur = s.get("dt").and_then(Value::as_f64).map(|ms| ms / 1000.0);
                // candidates list several credited artists; compare against each and the joined form
                let sc = std::iter::once(artists.clone())
                    .chain(artists.split(' ').map(str::to_string))
                    .map(|a| score(q, &a, &name, dur))
                    .max_by(|a, b| a.overall.total_cmp(&b.overall))?;
                sc.accepted(cfg.min_score).then_some((sc, id, name, artists))
            })
            .max_by(|a, b| a.0.overall.total_cmp(&b.0.overall));

        let Some((sc, id, name, artists)) = best else { return Ok(None) };

        let lyric: Value = http
            .get("https://music.163.com/api/song/lyric")
            .header("User-Agent", USER_AGENT)
            .header("Referer", "https://music.163.com/")
            .header("Cookie", "os=pc")
            .query(&[("id", id.to_string()), ("lv", "-1".into()), ("kv", "1".into()), ("tv", "-1".into())])
            .send()
            .await
            .context("netease lyric")?
            .error_for_status()?
            .json()
            .await
            .context("netease lyric: bad json")?;

        let text = lyric.pointer("/lrc/lyric").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Ok(None);
        }
        let (plain, synced) = if lrc::looks_synced(&text) { (Some(lrc::strip_timestamps(&text)), Some(text)) } else { (Some(text), None) };
        Ok(Some(Lyrics {
            plain,
            synced,
            source: "netease".into(),
            url: Some(format!("https://music.163.com/song?id={id}")),
            matched_artist: artists,
            matched_title: name,
            score: sc.overall,
        }))
    }
}
