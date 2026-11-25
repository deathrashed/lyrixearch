//! Lyrixearch plugin: lyrics from Metal Archives (www.metal-archives.com).
//!
//! The lookup is the one from metallum-nim, cut down to what finding lyrics needs:
//! 1. the advanced song search (`/search/ajax-advanced/searching/songs/`), a DataTables JSON
//!    reply with one row per song: band, release, type, title, and a lyrics link `lyricsLink_<id>`;
//! 2. the lyrics fragment for that id (`/release/ajax-view-lyrics/id/<id>`);
//! 3. the same clean-up of that fragment (line breaks kept, stanzas separated by one blank line).
//!
//! Requests go through `curl`, as metallum-nim's do. Protocol v1 (docs/PLUGINS.md): one JSON query
//! on stdin, one JSON reply on stdout.

use serde_json::{json, Value};
use std::io::Read;
use std::process::Command;

const BASE: &str = "https://www.metal-archives.com";
const UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

fn main() {
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    let reply = match serde_json::from_str::<Value>(&input) {
        Ok(q) => lookup(&q).unwrap_or_else(|e| e),
        Err(e) => json!({ "found": false, "error": format!("bad query: {e}") }),
    };
    println!("{reply}");
}

/// `Ok` is the reply; `Err` is a reply that reports a problem (also printed as is).
fn lookup(q: &Value) -> Result<Value, Value> {
    let text = |k: &str| q.get(k).and_then(Value::as_str).unwrap_or("").trim().to_string();
    let (artist, title, album) = (text("artist"), text("title"), text("album"));
    if artist.is_empty() || title.is_empty() {
        return Ok(json!({ "found": false }));
    }

    let want_band = norm(&artist);
    let want_title = norm(&plain_title(&title));
    let want_album = norm(&album);
    // Metal Archives keeps lyrics on the original release, while the same song also sits on many
    // live albums and compilations without them: ask for the album first when it is known, then
    // the exact band and title, then a looser search
    let mut rows = if album.is_empty() { Vec::new() } else { search(&artist, &title, Some(&album), true)? };
    if rows.is_empty() {
        rows = search(&artist, &title, None, true)?;
    }
    if rows.is_empty() {
        rows = search(&artist, &title, None, false)?;
    }
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
    // the same song is often on many releases (live albums, compilations): try the best few
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut found = None;
    for (score, song) in candidates.into_iter().take(6) {
        let fragment = get(&format!("{BASE}/release/ajax-view-lyrics/id/{}", song.lyric_id), "text/html, */*; q=0.01")?;
        let lyrics = clean(&fragment);
        if !unavailable(&lyrics) {
            found = Some((score, song, lyrics));
            break;
        }
    }
    let Some((score, song, lyrics)) = found else { return Ok(json!({ "found": false })) };
    Ok(json!({
        "found": true,
        "plain": lyrics,
        "artist": song.band,
        "title": song.title,
        "url": song.release_url,
        "score": score.min(0.99),
    }))
}

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
        // every row has the same lyrics icon; whether there is text only shows when it is fetched
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

/// The song search's rows (each an array of HTML cells).
fn search(artist: &str, title: &str, album: Option<&str>, exact: bool) -> Result<Vec<Value>, Value> {
    let flags = if exact { "&exactSongMatch=1&exactBandMatch=1" } else { "" };
    let release = match album {
        Some(a) => format!("{}&exactReleaseMatch=1", enc(&plain_title(a))),
        None => String::new(),
    };
    let url = format!(
        "{BASE}/search/ajax-advanced/searching/songs/?songTitle={}&bandName={}{flags}&releaseTitle={release}&lyrics=&genre=&sEcho=1&iDisplayStart=0&iDisplayLength=100",
        enc(&plain_title(title)),
        enc(artist)
    );
    let body = get(&url, "application/json, text/javascript, */*; q=0.01")?;
    let data: Value = serde_json::from_str(&body).map_err(|_| problem("the song search did not answer with JSON", None))?;
    Ok(data.get("aaData").and_then(Value::as_array).cloned().unwrap_or_default())
}

/// GET through curl. Problems come back as replies Lyrixearch understands: a refusal or a
/// "slow down" asks it to wait before the next lookup.
fn get(url: &str, accept: &str) -> Result<String, Value> {
    let out = Command::new("curl")
        .args(["-sS", "-L", "--compressed", "--max-time", "25", "-A", UA])
        .args(["-H", &format!("Accept: {accept}"), "-H", "Accept-Language: en-US,en;q=0.9", "-H", "X-Requested-With: XMLHttpRequest"])
        .args(["-H", "Referer: https://www.metal-archives.com/search/advanced/searching/songs", "-w", "\n%{http_code}", url])
        .output()
        .map_err(|e| problem(&format!("could not run curl: {e}"), None))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(problem(&format!("request failed: {}", err.trim()), Some(60)));
    }
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let (body, code) = text.rsplit_once('\n').unwrap_or((&text, "0"));
    let code: u16 = code.trim().parse().unwrap_or(0);
    let challenged = body.contains("Just a moment") || body.contains("cf-chl") || body.contains("challenge-platform");
    match code {
        200..=299 if !challenged => Ok(body.to_string()),
        429 => Err(problem("Metal Archives asked to slow down", Some(180))),
        _ if challenged => Err(problem("Metal Archives answered with a Cloudflare check", Some(600))),
        403 | 503 => Err(problem(&format!("Metal Archives refused the request (HTTP {code})"), Some(300))),
        _ => Err(problem(&format!("HTTP {code}"), None)),
    }
}

fn problem(why: &str, retry_after: Option<u64>) -> Value {
    match retry_after {
        Some(s) => json!({ "found": false, "error": why, "retry_after": s }),
        None => json!({ "found": false, "error": why }),
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
    let s = decode(&strip_tags(&s)).replace("\r\n", "\n").replace('\r', "\n");
    // runs of 3+ newlines are stanza breaks; a double newline is one line break
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
    let lines: Vec<String> = out.lines().map(|l| l.trim().to_string()).collect();
    let mut kept: Vec<String> = Vec::new();
    for line in lines {
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

fn decode(s: &str) -> String {
    let mut out = s.replace("&nbsp;", " ").replace("&quot;", "\"").replace("&#039;", "'").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">");
    // numeric entities (&#8217; and the like)
    while let Some(start) = out.find("&#") {
        let rest = &out[start + 2..];
        let Some(end) = rest.find(';') else { break };
        let num = &rest[..end];
        let code = if let Some(hex) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(hex, 16).ok() } else { num.parse().ok() };
        let ch = code.and_then(char::from_u32).map(String::from).unwrap_or_default();
        out.replace_range(start..start + 2 + end + 1, &ch);
    }
    out.replace("&amp;", "&")
}

fn cell_text(html: &str) -> String {
    decode(&strip_tags(html)).split_whitespace().collect::<Vec<_>>().join(" ")
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

fn enc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_search_row() {
        let row = json!([
            "<a href=\"https://www.metal-archives.com/bands/Metallica/125\">Metallica</a>",
            "<a href=\"https://www.metal-archives.com/albums/Metallica/Ride_the_Lightning/547\">Ride the Lightning</a>",
            "Full-length",
            "Fade to Black",
            "<a href=\"javascript:;\" id=\"lyricsLink_3449\" title=\"Toggle lyrics display\">Show lyrics</a>"
        ]);
        let icon = json!(["<a href=\"x\">Metallica</a>", "<a href=\"y\">Live</a>", "Live album", "Fade to Black",
            "<a href=\"javascript:;\" id=\"lyricsLink_6888961\"><span class=\"ui-icon\">Edit song lyrics</span></a>"]);
        assert_eq!(Song::from_row(&icon).unwrap().kind, "Live album");
        let s = Song::from_row(&row).unwrap();
        assert_eq!((s.band.as_str(), s.release.as_str(), s.title.as_str(), s.lyric_id.as_str()), ("Metallica", "Ride the Lightning", "Fade to Black", "3449"));
        assert!(s.release_url.ends_with("/547"));
    }

    #[test]
    fn a_row_without_lyrics_is_skipped() {
        let row = json!(["<a href=\"x\">Band</a>", "<a href=\"y\">Album</a>", "Demo", "Song", ""]);
        assert!(Song::from_row(&row).is_none());
    }

    #[test]
    fn cleans_the_fragment_like_metallum_nim() {
        let html = "Life it seems will fade away<br />\r\nDrifting further every day<br />\r\n<br />\r\n<br />\r\nI have lost the will to live&#8230;<br />\r\n";
        assert_eq!(clean(html), "Life it seems will fade away\nDrifting further every day\n\nI have lost the will to live…");
        assert!(unavailable(&clean("<em>(lyrics not available)</em>")));
    }

    #[test]
    fn titles_and_names_compare_loosely() {
        assert_eq!(norm("Ride the Lightning!"), norm("ride-the lightning"));
        assert_eq!(plain_title("Fade to Black (Remastered 2016)"), "Fade to Black");
        assert_eq!(enc("Mötley Crüe"), "M%C3%B6tley+Cr%C3%BCe");
    }
}
