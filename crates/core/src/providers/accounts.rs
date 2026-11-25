//! Sources that need the person's own account: Spotify, Apple Music and Deezer.
//!
//! None of them offers lyrics through a public API; their web players show lyrics to signed-in
//! listeners. The desktop app keeps one hidden window per service, signed in by the person on the
//! service's own page (the app never sees a password), and answers [`SessionBridge`] calls by
//! running a small script there that uses the service's own web client. This module does the rest:
//! choosing the track with the usual match scoring and turning the reply into lyrics.
//! Without the desktop app (the CLI) there is no session, and the source says so.

use super::Provider;
use crate::config::Config;
use crate::matching::score;
use crate::model::{Lyrics, Query};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// Runs an operation in the signed-in page of a service and returns its JSON answer.
#[async_trait]
pub trait SessionBridge: Send + Sync {
    async fn call(&self, service: &str, op: &str, args: Value) -> Result<Value>;
}

static BRIDGE: once_cell::sync::OnceCell<Arc<dyn SessionBridge>> = once_cell::sync::OnceCell::new();

/// Register the desktop app's bridge (once per process).
pub fn set_bridge(bridge: Arc<dyn SessionBridge>) {
    let _ = BRIDGE.set(bridge);
}

/// The account services, as `(id, name)`.
pub const SERVICES: &[(&str, &str)] = &[("spotify", "Spotify"), ("apple_music", "Apple Music"), ("deezer", "Deezer")];

pub struct AccountSource {
    pub id: &'static str,
    pub name: &'static str,
}

/// One search result as the page script reports it.
#[derive(Debug, Clone, serde::Deserialize)]
struct Candidate {
    id: String,
    title: String,
    artist: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    duration_ms: Option<f64>,
    #[serde(default)]
    url: Option<String>,
}

#[async_trait]
impl Provider for AccountSource {
    fn id(&self) -> &str {
        self.id
    }

    async fn fetch(&self, _http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let bridge = BRIDGE.get().ok_or_else(|| anyhow!("{} needs the desktop app: sign in under Settings → Accounts", self.name))?;
        if q.artist.trim().is_empty() || q.title.trim().is_empty() {
            return Ok(None);
        }
        let found = bridge.call(self.id, "search", json!({ "artist": q.artist, "title": q.title, "album": q.album })).await?;
        let candidates: Vec<Candidate> = serde_json::from_value(found.get("tracks").cloned().unwrap_or(Value::Array(Vec::new())))?;
        let Some((best, sc)) = pick(q, &candidates, cfg.min_score) else { return Ok(None) };
        let reply = bridge.call(self.id, "lyrics", json!({ "id": best.id })).await?;
        let Some((plain, synced)) = read_lyrics(&reply)? else { return Ok(None) };
        Ok(Some(Lyrics {
            plain,
            synced,
            source: self.id.to_string(),
            url: best.url.clone(),
            matched_artist: best.artist.clone(),
            matched_title: best.title.clone(),
            score: sc,
        }))
    }
}

/// The best candidate that passes the usual match check, with its score.
fn pick<'a>(q: &Query, candidates: &'a [Candidate], min_score: f32) -> Option<(&'a Candidate, f32)> {
    let want_album = q.album.as_deref().map(norm).unwrap_or_default();
    candidates
        .iter()
        .filter_map(|c| {
            let sc = score(q, &c.artist, &c.title, c.duration_ms.map(|ms| ms / 1000.0));
            if !sc.accepted(min_score) {
                return None;
            }
            let bonus = if !want_album.is_empty() && norm(&c.album) == want_album { 0.02 } else { 0.0 };
            Some((c, (sc.overall + bonus).min(1.0)))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

/// A lyrics reply: `{ "lines": [{ "ms": 1234, "text": "…" }], "synced": true }`, or `{ "ttml": "…" }`
/// (Apple Music), or `{ "plain": "…" }`. `None` when the service has no lyrics for the track.
fn read_lyrics(reply: &Value) -> Result<Option<(Option<String>, Option<String>)>> {
    if let Some(err) = reply.get("error").and_then(Value::as_str) {
        bail!("{err}");
    }
    if let Some(ttml) = reply.get("ttml").and_then(Value::as_str) {
        return Ok(from_ttml(ttml));
    }
    if let Some(lines) = reply.get("lines").and_then(Value::as_array) {
        let synced = reply.get("synced").and_then(Value::as_bool).unwrap_or(false);
        let rows: Vec<(Option<u64>, String)> = lines
            .iter()
            .map(|l| (l.get("ms").and_then(Value::as_f64).map(|m| m.max(0.0) as u64), l.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string()))
            .collect();
        return Ok(from_lines(&rows, synced));
    }
    if let Some(plain) = reply.get("plain").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty()) {
        return Ok(Some((Some(plain.to_string()), None)));
    }
    Ok(None)
}

/// Lines with start times -> plain text and, when timed, LRC.
fn from_lines(rows: &[(Option<u64>, String)], synced: bool) -> Option<(Option<String>, Option<String>)> {
    let plain = rows.iter().map(|(_, t)| clean_line(t)).collect::<Vec<_>>().join("\n").trim().to_string();
    if plain.is_empty() {
        return None;
    }
    let lrc = (synced && rows.iter().all(|(ms, _)| ms.is_some()))
        .then(|| rows.iter().map(|(ms, t)| format!("{}{}", stamp(ms.unwrap_or(0)), clean_line(t))).collect::<Vec<_>>().join("\n"));
    Some((Some(plain), lrc))
}

/// Spotify marks breaks with "♪"; an empty line reads better.
fn clean_line(t: &str) -> String {
    let t = t.trim();
    if t == "♪" {
        String::new()
    } else {
        t.to_string()
    }
}

fn stamp(ms: u64) -> String {
    format!("[{:02}:{:02}.{:02}]", ms / 60_000, (ms / 1000) % 60, (ms % 1000) / 10)
}

/// Apple Music's TTML: `<p begin="12.345" …>line</p>` per line (begin may be `m:ss.mmm` or
/// `h:mm:ss.mmm`), `<div>` per verse. Without `begin` the lyrics are unsynced.
fn from_ttml(ttml: &str) -> Option<(Option<String>, Option<String>)> {
    let mut rows: Vec<(Option<u64>, String)> = Vec::new();
    let mut rest = ttml;
    let mut first_div = true;
    while let Some(i) = rest.find(['<']) {
        rest = &rest[i..];
        if rest.starts_with("<div") {
            if !first_div && rows.last().is_some_and(|(_, t)| !t.is_empty()) {
                rows.push((rows.last().and_then(|r| r.0), String::new()));
            }
            first_div = false;
        }
        if rest.starts_with("<p") && !rest.starts_with("<p/") {
            let Some(open_end) = rest.find('>') else { break };
            let tag = &rest[..open_end];
            let Some(close) = rest.find("</p>") else { break };
            let inner = &rest[open_end + 1..close];
            let text: String = html_escape::decode_html_entities(&strip_tags(inner)).split_whitespace().collect::<Vec<_>>().join(" ");
            rows.push((attr(tag, "begin").and_then(parse_clock), text));
            rest = &rest[close + 4..];
            continue;
        }
        rest = &rest[1..];
    }
    let synced = !rows.is_empty() && rows.iter().filter(|(_, t)| !t.is_empty()).all(|(ms, _)| ms.is_some());
    from_lines(&rows, synced)
}

fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    Some(&tag[i..i + tag[i..].find('"')?])
}

/// "12.345", "1:02.345", "1:02:03.5", "12.3s" -> milliseconds.
fn parse_clock(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('s');
    let mut total = 0.0;
    for part in s.split(':') {
        total = total * 60.0 + part.parse::<f64>().ok()?;
    }
    Some((total * 1000.0).round() as u64)
}

fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(artist: &str, title: &str, album: Option<&str>) -> Query {
        Query { artist: artist.into(), title: title.into(), album: album.map(String::from), ..Default::default() }
    }

    #[test]
    fn picks_the_matching_track_preferring_the_album() {
        let c: Vec<Candidate> = serde_json::from_value(json!([
            { "id": "a", "title": "Creep", "artist": "Stone Temple Pilots", "album": "Core" },
            { "id": "b", "title": "Creep", "artist": "Radiohead", "album": "Creep (Live)" },
            { "id": "c", "title": "Creep", "artist": "Radiohead", "album": "Pablo Honey" }
        ]))
        .unwrap();
        let (best, _) = pick(&q("Radiohead", "Creep", Some("Pablo Honey")), &c, 0.8).unwrap();
        assert_eq!(best.id, "c");
        assert!(pick(&q("Radiohead", "Karma Police", None), &c, 0.8).is_none());
    }

    #[test]
    fn timed_lines_become_lrc() {
        let reply = json!({ "synced": true, "lines": [{ "ms": 19160, "text": "When you were here before" }, { "ms": 24090, "text": "♪" }, { "ms": 61500, "text": "Couldn't look you in the eye" }] });
        let (plain, lrc) = read_lyrics(&reply).unwrap().unwrap();
        assert_eq!(plain.unwrap(), "When you were here before\n\nCouldn't look you in the eye");
        assert_eq!(lrc.unwrap(), "[00:19.16]When you were here before\n[00:24.09]\n[01:01.50]Couldn't look you in the eye");
        let (_, none) = read_lyrics(&json!({ "synced": false, "lines": [{ "text": "a" }] })).unwrap().unwrap();
        assert!(none.is_none());
        assert!(read_lyrics(&json!({ "lines": [] })).unwrap().is_none());
        assert!(read_lyrics(&json!({ "error": "not signed in" })).is_err());
    }

    #[test]
    fn apple_ttml_becomes_lrc_with_verses() {
        let ttml = r#"<tt><body><div begin="1.0"><p begin="00:12.300" end="00:15.0">When you were here before</p><p begin="15.900">Couldn&apos;t look you in the eye</p></div><div><p begin="1:02.5">You're just like an angel</p></div></body></tt>"#;
        let (plain, lrc) = from_ttml(ttml).unwrap();
        assert_eq!(plain.unwrap(), "When you were here before\nCouldn't look you in the eye\n\nYou're just like an angel");
        assert_eq!(
            lrc.unwrap(),
            "[00:12.30]When you were here before\n[00:15.90]Couldn't look you in the eye\n[00:15.90]\n[01:02.50]You're just like an angel"
        );
        let (_, unsynced) = from_ttml("<tt><body><div><p>Line one</p><p>Line two</p></div></body></tt>").unwrap();
        assert!(unsynced.is_none());
    }
}
