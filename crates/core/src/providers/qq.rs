//! QQ Music's public web endpoints (unofficial, no key). Returns synced lyrics.

use super::web::BROWSER_UA;
use super::Provider;
use crate::config::Config;
use crate::lrc;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use serde_json::Value;

pub struct QqMusic;

#[async_trait]
impl Provider for QqMusic {
    fn id(&self) -> &'static str {
        "qqmusic"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let search: Value = http
            .get("https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg")
            .header("User-Agent", BROWSER_UA)
            .header("Referer", "https://y.qq.com/")
            .query(&[("key", format!("{} {}", q.artist, q.title)), ("format", "json".into())])
            .send()
            .await
            .context("qq search")?
            .error_for_status()?
            .json()
            .await
            .context("qq search: bad json")?;

        let items = search.pointer("/data/song/itemlist").and_then(Value::as_array).cloned().unwrap_or_default();
        let best = items
            .iter()
            .filter_map(|s| {
                let mid = s.get("mid")?.as_str()?.to_string();
                let name = s.get("name")?.as_str()?.to_string();
                let singer = s.get("singer").and_then(Value::as_str).unwrap_or("").to_string();
                let sc = score(q, &singer, &name, None);
                sc.accepted(cfg.min_score).then_some((sc.overall, mid, name, singer))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((overall, mid, name, singer)) = best else { return Ok(None) };

        let lyric: Value = http
            .get("https://c.y.qq.com/lyric/fcgi-bin/fcg_query_lyric_new.fcg")
            .header("User-Agent", BROWSER_UA)
            .header("Referer", "https://y.qq.com/portal/player.html")
            .query(&[("songmid", mid.as_str()), ("format", "json"), ("nobase64", "1"), ("g_tk", "5381")])
            .send()
            .await
            .context("qq lyric")?
            .error_for_status()?
            .json()
            .await
            .context("qq lyric: bad json")?;

        let text = lyric.get("lyric").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if text.is_empty() {
            return Ok(None);
        }
        let text = html_escape::decode_html_entities(&text).into_owned();
        let (plain, synced) = if lrc::looks_synced(&text) { (Some(lrc::strip_timestamps(&text)), Some(text)) } else { (Some(text), None) };
        Ok(Some(Lyrics {
            plain,
            synced,
            source: "qqmusic".into(),
            url: Some(format!("https://y.qq.com/n/ryqq/songDetail/{mid}")),
            matched_artist: singer,
            matched_title: name,
            score: overall,
        }))
    }
}
