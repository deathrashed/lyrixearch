//! Last.fm tag correction (`track.getCorrection`): fixes misspelled or alternate artist and
//! title names before searching. Only used when the user has set `lastfm_api_key`.

use anyhow::{Context, Result};
use serde_json::Value;

const API: &str = "https://ws.audioscrobbler.com/2.0/";

/// The corrected `(artist, title)`, or `None` when Last.fm has no correction (or returns the
/// same names). Errors never contain the API key.
pub async fn correct(http: &reqwest::Client, api_key: &str, artist: &str, title: &str) -> Result<Option<(String, String)>> {
    let resp = http
        .get(API)
        .query(&[("method", "track.getCorrection"), ("artist", artist), ("track", title), ("api_key", api_key), ("format", "json")])
        .header("User-Agent", crate::providers::USER_AGENT)
        .send()
        .await
        // the request URL carries the key: never put it in an error message
        .map_err(|e| e.without_url())
        .context("last.fm correction")?;
    let status = resp.status();
    let body: Value = resp.json().await.map_err(|e| e.without_url()).context("last.fm correction: bad json")?;
    if !status.is_success() {
        let msg = body.get("message").and_then(Value::as_str).unwrap_or("request failed");
        anyhow::bail!("last.fm correction: HTTP {} ({msg})", status.as_u16());
    }
    Ok(parse_correction(&body, artist, title))
}

/// Read a `track.getCorrection` reply. Last.fm answers "no correction" with an empty string or
/// an object without a track, so anything unexpected means `None`.
pub fn parse_correction(body: &Value, artist: &str, title: &str) -> Option<(String, String)> {
    let track = body.pointer("/corrections/correction/track")?;
    let new_title = track.get("name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
    let new_artist = track.pointer("/artist/name").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty())?;
    let same = new_artist.eq_ignore_ascii_case(artist.trim()) && new_title.eq_ignore_ascii_case(title.trim());
    (!same).then(|| (new_artist.to_string(), new_title.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_correction() {
        let body = json!({"corrections": {"correction": {"track": {"name": "Bohemian Rhapsody", "artist": {"name": "Queen"}}, "@attr": {"index": "0"}}}});
        assert_eq!(parse_correction(&body, "queen", "bohemain rapsody"), Some(("Queen".into(), "Bohemian Rhapsody".into())));
    }

    #[test]
    fn no_correction_shapes_are_none() {
        assert_eq!(parse_correction(&json!({"corrections": "\n "}), "a", "b"), None);
        assert_eq!(parse_correction(&json!({"corrections": {}}), "a", "b"), None);
        assert_eq!(parse_correction(&json!({"error": 6, "message": "Track not found"}), "a", "b"), None);
        // the same names, only in a different case, is not a correction
        let same = json!({"corrections": {"correction": {"track": {"name": "song", "artist": {"name": "ARTIST"}}}}});
        assert_eq!(parse_correction(&same, "Artist", "Song"), None);
    }
}
