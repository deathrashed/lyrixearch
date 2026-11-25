//! Kugou's public web endpoints (unofficial, no key). Asks for the plain LRC format so no
//! proprietary decoding is needed.

use super::web::BROWSER_UA;
use super::CheckStatus;
use super::Provider;
use crate::config::Config;
use crate::lrc;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine as _;
use serde_json::Value;

pub struct Kugou;

async fn get_json(http: &reqwest::Client, url: &str, query: &[(&str, String)], ctx: &'static str) -> Result<Value> {
    http.get(url)
        .header("User-Agent", BROWSER_UA)
        .header("Referer", "https://www.kugou.com/")
        .query(query)
        .send()
        .await
        .context(ctx)?
        .check_status("kugou")?
        .json()
        .await
        .with_context(|| format!("{ctx}: bad json"))
}

#[async_trait]
impl Provider for Kugou {
    fn id(&self) -> &'static str {
        "kugou"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        // Kugou treats `+` and `%20` differently in the keyword (`+` returns a near-empty list),
        // so build this URL by hand instead of via `.query()`.
        let url = format!(
            "https://mobileservice.kugou.com/api/v3/search/song?keyword={}&page=1&pagesize=10&format=json",
            urlencoding::encode(&format!("{} {}", q.artist, q.title))
        );
        let search = get_json(http, &url, &[], "kugou search").await?;
        if let Some(code) = search.get("errcode").and_then(Value::as_i64) {
            if code != 0 {
                anyhow::bail!("kugou api errcode {code}");
            }
        }
        let infos = search.pointer("/data/info").and_then(Value::as_array).cloned().unwrap_or_default();
        let best = infos
            .iter()
            .filter_map(|s| {
                let hash = s.get("hash")?.as_str()?.to_string();
                let name = s.get("songname")?.as_str()?.to_string();
                let singer = s.get("singername").and_then(Value::as_str).unwrap_or("").to_string();
                let dur = s.get("duration").and_then(Value::as_f64);
                let sc = score(q, &singer, &name, dur);
                sc.accepted(cfg.min_score).then_some((sc.overall, hash, name, singer))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((overall, hash, name, singer)) = best else { return Ok(None) };

        let cands = get_json(
            http,
            "https://krcs.kugou.com/search",
            &[("ver", "1".into()), ("man", "yes".into()), ("client", "mobi".into()), ("hash", hash.clone())],
            "kugou lyric search",
        )
        .await?;
        let cand = cands.get("candidates").and_then(Value::as_array).and_then(|a| a.first()).cloned();
        let Some(cand) = cand else { return Ok(None) };
        let (Some(id), Some(key)) =
            (cand.get("id").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())), cand.get("accesskey").and_then(Value::as_str))
        else {
            return Ok(None);
        };

        let dl = get_json(
            http,
            "https://lyrics.kugou.com/download",
            &[("ver", "1".into()), ("client", "pc".into()), ("id", id), ("accesskey", key.to_string()), ("fmt", "lrc".into()), ("charset", "utf8".into())],
            "kugou lyric download",
        )
        .await?;
        let content = dl.get("content").and_then(Value::as_str).unwrap_or("");
        if content.is_empty() {
            return Ok(None);
        }
        let bytes = base64::engine::general_purpose::STANDARD.decode(content).context("kugou: bad base64")?;
        let text = String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').trim().to_string();
        if text.is_empty() {
            return Ok(None);
        }
        let (plain, synced) = if lrc::looks_synced(&text) { (Some(lrc::strip_timestamps(&text)), Some(text)) } else { (Some(text), None) };
        Ok(Some(Lyrics {
            plain,
            synced,
            source: "kugou".into(),
            url: Some(format!("https://www.kugou.com/song/#hash={hash}")),
            matched_artist: singer,
            matched_title: name,
            score: overall,
        }))
    }
}
