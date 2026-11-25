//! Metal Archives (www.metal-archives.com): the lyrics lookup from metallum-nim.
//!
//! 1. The advanced song search (`/search/ajax-advanced/searching/songs/`), a DataTables JSON reply
//!    with one row per song: band, release, type, title, and a lyrics link `lyricsLink_<id>`.
//! 2. The lyrics fragment for that id (`/release/ajax-view-lyrics/id/<id>`).
//! 3. metallum-nim's clean-up of the fragment (line breaks kept, one blank line between stanzas).
//!
//! The site keeps lyrics on a song's original release, while the same song also sits on many live
//! albums and compilations without them, so the album is searched first when it is known, and the
//! best few releases are tried. A Cloudflare check is answered through the app's browser like any
//! other source's.

use super::web::get_text;
use super::Provider;
use crate::config::Config;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

pub struct MetalArchives;

const BASE: &str = "https://www.metal-archives.com";
const HEADERS: &[(&str, &str)] = &[
    ("Accept", "application/json, text/javascript, */*; q=0.01"),
    ("X-Requested-With", "XMLHttpRequest"),
    ("Referer", "https://www.metal-archives.com/search/advanced/searching/songs"),
];

struct Song {
    band: String,
    release: String,
    /// Full-length, EP, Live album, ...
    kind: String,
    release_url: String,
    title: String,
    lyric_id: String,
}

impl Song {
    /// A search row: `[band, release, type, title, lyrics link]` (HTML cells).
    fn from_row(row: &Value) -> Option<Self> {
        let cells: Vec<&str> = row.as_array()?.iter().map(|c| c.as_str().unwrap_or("")).collect();
        if cells.len() < 4 {
            return None;
        }
        let last = cells[cells.len() - 1];
        // every row carries the same lyrics icon; whether there is text only shows when it is fetched
        let lyric_id = after(last, "lyricsLink_").map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>()).filter(|id| !id.is_empty())?;
        Some(Song {
            band: cell_text(cells[0]),
            release: cell_text(cells[1]),
            release_url: href(cells[1]).unwrap_or_default(),
            kind: if cells.len() >= 5 { cell_text(cells[2]) } else { String::new() },
            title: cell_text(cells[cells.len() - 2]),
            lyric_id,
        })
    }
}

async fn search(http: &reqwest::Client, artist: &str, title: &str, album: Option<&str>, exact: bool) -> Result<Vec<Value>> {
    let flags = if exact { "&exactSongMatch=1&exactBandMatch=1" } else { "" };
    let release = album.map(|a| format!("{}&exactReleaseMatch=1", urlencoding::encode(&plain_title(a)))).unwrap_or_default();
    let url = format!(
        "{BASE}/search/ajax-advanced/searching/songs/?songTitle={}&bandName={}{flags}&releaseTitle={release}&lyrics=&genre=&sEcho=1&iDisplayStart=0&iDisplayLength=100",
        urlencoding::encode(&plain_title(title)),
        urlencoding::encode(artist)
    );
    let body = get_text(http, &url, HEADERS).await?;
    let data: Value = serde_json::from_str(body.trim()).map_err(|_| anyhow::anyhow!("the song search did not answer with JSON"))?;
    Ok(data.get("aaData").and_then(Value::as_array).cloned().unwrap_or_default())
}

#[async_trait]
impl Provider for MetalArchives {
    fn id(&self) -> &'static str {
        "metal_archives"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, _cfg: &Config) -> Result<Option<Lyrics>> {
        if q.artist.trim().is_empty() || q.title.trim().is_empty() {
            return Ok(None);
        }
        let album = q.album.as_deref().map(str::trim).filter(|a| !a.is_empty());
        let mut rows = match album {
            Some(a) => search(http, &q.artist, &q.title, Some(a), true).await?,
            None => Vec::new(),
        };
        if rows.is_empty() {
            rows = search(http, &q.artist, &q.title, None, true).await?;
        }
        if rows.is_empty() {
            rows = search(http, &q.artist, &q.title, None, false).await?;
        }

        let want_band = norm(&q.artist);
        let want_title = norm(&plain_title(&q.title));
        let want_album = album.map(norm).unwrap_or_default();
        let mut candidates: Vec<(f32, Song)> = Vec::new();
        for row in rows {
            let Some(song) = Song::from_row(&row) else { continue };
            let band = norm(&song.band);
            let band_ok = band == want_band;
            let band_close = !band_ok && !band.is_empty() && (band.contains(&want_band) || want_band.contains(&band));
            if !(band_ok || band_close) || norm(&plain_title(&song.title)) != want_title {
                continue;
            }
            let mut score: f32 = if band_ok { 0.95 } else { 0.86 };
            if !want_album.is_empty() && norm(&song.release) == want_album {
                score += 0.04;
            } else if song.kind == "Full-length" {
                score += 0.01;
            }
            candidates.push((score, song));
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (score, song) in candidates.into_iter().take(6) {
            let fragment = get_text(http, &format!("{BASE}/release/ajax-view-lyrics/id/{}", song.lyric_id), HEADERS).await?;
            let text = clean(&fragment);
            if unavailable(&text) {
                continue;
            }
            return Ok(Some(Lyrics {
                plain: Some(text),
                source: "metal_archives".into(),
                url: Some(song.release_url),
                matched_artist: song.band,
                matched_title: song.title,
                score: score.min(0.99),
                ..Default::default()
            }));
        }
        Ok(None)
    }
}

/// metallum-nim's `stripHtmlPreservingBreaks`: each lyric line ends in `<br />` plus CRLF, stanza
/// breaks carry more; the result has single line breaks and one blank line between stanzas.
fn clean(html: &str) -> String {
    let mut s = html.to_string();
    for br in ["<br />", "<br/>", "<br>", "<BR />", "<BR>"] {
        s = s.replace(br, "\n");
    }
    s = s.replace("</p>", "\n\n");
    let s = html_escape::decode_html_entities(&strip_tags(&s)).replace("\r\n", "\n").replace('\r', "\n");
    let mut out = String::new();
    let mut run = 0;
    for ch in s.chars() {
        if ch == '\n' {
            run += 1;
            continue;
        }
        if run > 0 {
            out.push_str(if run >= 3 { "\n\n" } else { "\n" });
            run = 0;
        }
        out.push(ch);
    }
    let mut kept: Vec<String> = Vec::new();
    for line in out.lines().map(|l| l.trim().to_string()) {
        if line.is_empty() && kept.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        kept.push(line);
    }
    while kept.last().is_some_and(|l| l.is_empty()) {
        kept.pop();
    }
    kept.join("\n")
}

fn unavailable(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    t.is_empty() || t.contains("lyrics are not available") || t == "(lyrics not available)" || t == "lyrics not available" || t == "(instrumental)"
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
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

fn cell_text(html: &str) -> String {
    html_escape::decode_html_entities(&strip_tags(html)).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn href(html: &str) -> Option<String> {
    let rest = after(html, "href=\"")?;
    Some(rest[..rest.find('"')?].to_string())
}

fn after<'a>(s: &'a str, marker: &str) -> Option<&'a str> {
    s.find(marker).map(|i| &s[i + marker.len()..])
}

/// Lowercase letters and digits only, so "Ride the Lightning" and "ride-the lightning" match.
fn norm(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

/// The title without a trailing "(Remastered 2016)" or "[Live]".
fn plain_title(s: &str) -> String {
    let t = s.trim();
    match t.rfind(['(', '[']) {
        Some(i) if i > 0 && t.ends_with([')', ']']) => t[..i].trim().to_string(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_search_row() {
        let row = json!([
            "<a href=\"https://www.metal-archives.com/bands/Metallica/125\">Metallica</a>",
            "<a href=\"https://www.metal-archives.com/albums/Metallica/Ride_the_Lightning/547\">Ride the Lightning</a>",
            "Full-length",
            "Fade to Black",
            "<a href=\"javascript:;\" id=\"lyricsLink_3449\" title=\"Toggle lyrics display\"><span class=\"ui-icon\">Edit song lyrics</span></a>"
        ]);
        let s = Song::from_row(&row).unwrap();
        assert_eq!(
            (s.band.as_str(), s.release.as_str(), s.kind.as_str(), s.title.as_str(), s.lyric_id.as_str()),
            ("Metallica", "Ride the Lightning", "Full-length", "Fade to Black", "3449")
        );
        assert!(s.release_url.ends_with("/547"));
        assert!(Song::from_row(&json!(["a", "b", "Demo", "Song", ""])).is_none(), "no lyrics link, no row");
    }

    #[test]
    fn cleans_the_fragment_like_metallum_nim() {
        let html =
            "Life it seems will fade away<br />\r\nDrifting further every day<br />\r\n<br />\r\n<br />\r\nI have lost the will to live&#8230;<br />\r\n";
        assert_eq!(clean(html), "Life it seems will fade away\nDrifting further every day\n\nI have lost the will to live…");
        assert!(unavailable(&clean("\n\t<em>(lyrics not available)</em>\n")));
    }

    #[test]
    fn names_compare_loosely() {
        assert_eq!(norm("Ride the Lightning!"), norm("ride-the lightning"));
        assert_eq!(plain_title("Fade to Black (Remastered 2016)"), "Fade to Black");
    }
}
