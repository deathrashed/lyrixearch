//! AI transcription: when no source has the lyrics, write them down from the track itself.
//!
//! The audio is the person's own file. Engines:
//! - `local`: whisper.cpp (`whisper-cli`) with a model file, offline and free; `ffmpeg` turns the
//!   track into the 16 kHz WAV it reads.
//! - `groq` / `openai`: their transcription API with the person's own key (Groq has a free tier).
//!
//! A transcription can be wrong (singing over music is hard to hear), so results are marked for
//! review: a search finds them, but Fetch does not write them into the file by itself.

use super::Provider;
use crate::config::Config;
use crate::model::{Lyrics, Query};
use anyhow::{anyhow, bail, Context, Result};
use async_trait::async_trait;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

pub struct Transcribe;

/// The recommended local model (whisper.cpp's large-v3-turbo, 5-bit: good quality, about 550 MB).
pub const MODEL_NAME: &str = "ggml-large-v3-turbo-q5_0.bin";
pub const MODEL_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin";
/// The cloud APIs take files up to 25 MB; bigger tracks are made smaller with ffmpeg first.
const UPLOAD_LIMIT: u64 = 24 * 1024 * 1024;

/// Where downloaded models live: `<config dir>/models`.
pub fn models_dir() -> PathBuf {
    Config::config_dir().unwrap_or_else(std::env::temp_dir).join("models")
}

/// The model file to use: the configured one, else any model in the models folder.
pub fn model_path(cfg: &Config) -> Option<PathBuf> {
    if let Some(p) = cfg.whisper_model.as_deref().filter(|p| !p.trim().is_empty()) {
        return Some(PathBuf::from(p));
    }
    let preferred = models_dir().join(MODEL_NAME);
    if preferred.is_file() {
        return Some(preferred);
    }
    std::fs::read_dir(models_dir())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().is_some_and(|x| x == "bin") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("ggml-")))
}

/// A program on PATH, or in the usual Homebrew and local folders (apps started from the Dock or a
/// launcher do not get the shell's PATH).
pub fn find_tool(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin"].iter().map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join(".local/bin"));
    }
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

pub fn whisper_bin(cfg: &Config) -> Option<PathBuf> {
    cfg.whisper_bin
        .as_deref()
        .filter(|p| !p.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| ["whisper-cli", "whisper-cpp", "whisper.cpp"].iter().find_map(|n| find_tool(n)))
}

#[async_trait]
impl Provider for Transcribe {
    fn id(&self) -> &'static str {
        "ai_transcribe"
    }

    /// Transcribing a song takes from a few seconds (cloud) to a minute or two (local, CPU only).
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(600))
    }

    fn timeouts_count(&self) -> bool {
        false
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let Some(path) = q.source_path.as_deref().map(PathBuf::from).filter(|p| p.is_file()) else { return Ok(None) };
        // the artist and title as vocabulary (names are what Whisper gets wrong most); not for
        // placeholder names, which would only mislead it
        let hint = crate::clean::unsuitable(&q.artist, &q.title).is_none().then(|| format!("{} - {}", q.artist.trim(), q.title.trim()));
        let segments = match cfg.transcribe_engine.as_str() {
            "groq" => cloud(http, &path, cfg, hint.as_deref(), "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo").await?,
            "openai" => cloud(http, &path, cfg, hint.as_deref(), "https://api.openai.com/v1/audio/transcriptions", "whisper-1").await?,
            _ => {
                let cfg = cfg.clone();
                tokio::task::spawn_blocking(move || local(&path, &cfg, hint.as_deref())).await.context("transcription stopped")??
            }
        };
        let rows = drop_loops(segments.into_iter().filter(|s| !noise(&s.text)).collect());
        if rows.iter().map(|s| s.text.split_whitespace().count()).sum::<usize>() < 8 {
            // an instrumental, or nothing intelligible
            return Ok(None);
        }
        // a rough transcription is enough to name the song: then the real lyrics are used, timed
        // by lining them up with the transcription
        if let Some(found) = identify(http, q, cfg, &rows).await {
            return Ok(Some(found));
        }
        let plain = rows.iter().map(|s| s.text.clone()).collect::<Vec<_>>().join("\n");
        let synced = rows
            .iter()
            .map(|s| {
                if s.words.is_empty() {
                    format!("{}{}", stamp(s.ms, '[', ']'), s.text)
                } else {
                    format!("{}{}", stamp(s.ms, '[', ']'), s.words.iter().map(|(ms, w)| format!("{}{w}", stamp(*ms, '<', '>'))).collect::<Vec<_>>().join(" "))
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        Ok(Some(Lyrics {
            plain: Some(plain),
            synced: Some(synced),
            source: "ai_transcribe".into(),
            url: None,
            matched_artist: q.artist.clone(),
            matched_title: q.title.clone(),
            score: 0.5,
            review: true,
        }))
    }
}

/// One line of the transcription: its start, its text, and its words' starts when known.
#[derive(Debug, Clone)]
struct Segment {
    ms: u64,
    text: String,
    words: Vec<(u64, String)>,
}

/// Whisper sometimes repeats one line over and over (a "loop"); a chorus repeats too, but not
/// this often in a row. Keep at most three of the same line in a row.
fn drop_loops(rows: Vec<Segment>) -> Vec<Segment> {
    let mut out: Vec<Segment> = Vec::new();
    let mut run = 0;
    for r in rows {
        if out.last().is_some_and(|l| norm(&l.text) == norm(&r.text)) {
            run += 1;
            if run >= 3 {
                continue;
            }
        } else {
            run = 0;
        }
        out.push(r);
    }
    out
}

fn norm(s: &str) -> String {
    s.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

/// Name the song from its words: Genius also searches lyrics, so the most distinctive transcribed
/// lines are looked up there. When a song comes up that matches the track's tags, its real lyrics
/// are used, timed against the transcription. When the track has no usable names (an "Unknown
/// Artist - Track 07"), the song the lines point to is taken, but marked for review.
async fn identify(http: &reqwest::Client, q: &Query, cfg: &Config, rows: &[Segment]) -> Option<Lyrics> {
    let named = crate::clean::unsuitable(&q.artist, &q.title).is_none();
    let mut lines: Vec<&str> = rows.iter().map(|s| s.text.as_str()).filter(|t| t.split_whitespace().count() >= 4).collect();
    lines.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let mut seen = std::collections::HashSet::new();
    lines.retain(|t| seen.insert(norm(t)));
    // (artist, title, url) -> how many lines pointed at it
    let mut votes: Vec<((String, String, String), usize)> = Vec::new();
    for line in lines.into_iter().take(3) {
        let Ok(hits) = super::genius::search_text(http, line).await else { continue };
        for hit in hits.into_iter().take(3) {
            match votes.iter_mut().find(|(h, _)| h.2 == hit.2) {
                Some((_, n)) => *n += 1,
                None => votes.push((hit, 1)),
            }
        }
    }
    let pick = if named {
        // must be the tagged song: names are more reliable than a transcription
        votes.into_iter().filter(|((a, t, _), _)| crate::matching::score(q, a, t, None).accepted(cfg.min_score)).max_by_key(|(_, n)| *n)
    } else {
        // no names to check against: at least two lines must agree
        votes.into_iter().filter(|(_, n)| *n >= 2).max_by_key(|(_, n)| *n)
    }?;
    let ((artist, title, url), _) = pick;
    let found = super::genius::lyrics_at(http, &url, &artist, &title, cfg).await.ok()??;
    let plain = found.embed_text()?;
    let synced = align(&plain, rows);
    Some(Lyrics {
        plain: Some(plain),
        synced,
        source: "ai_transcribe".into(),
        url: Some(url),
        matched_artist: artist,
        matched_title: title,
        score: if named { 0.9 } else { 0.6 },
        review: !named,
    })
}

/// Time the real lyrics with the transcription: the words of both are lined up (an edit-distance
/// alignment), each lyric line starts where its first matched word was heard, and lines without a
/// match get a time between their neighbours. `None` when too little matched to trust the timing.
fn align(lyrics: &str, rows: &[Segment]) -> Option<String> {
    // heard words with times; within a phrase without word times, spread evenly
    let mut heard: Vec<(String, u64)> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if !r.words.is_empty() {
            heard.extend(r.words.iter().map(|(ms, w)| (norm(w), *ms)));
            continue;
        }
        let words: Vec<&str> = r.text.split_whitespace().collect();
        let end = rows.get(i + 1).map(|n| n.ms).unwrap_or(r.ms + 3000).max(r.ms + 1);
        let step = (end - r.ms) / words.len().max(1) as u64;
        heard.extend(words.iter().enumerate().map(|(k, w)| (norm(w), r.ms + step * k as u64)));
    }
    heard.retain(|(w, _)| !w.is_empty());
    let lines: Vec<&str> = lyrics.lines().collect();
    let mut sung: Vec<(String, usize)> = Vec::new();
    for (li, l) in lines.iter().enumerate() {
        sung.extend(l.split_whitespace().map(|w| (norm(w), li)).filter(|(w, _)| !w.is_empty()));
    }
    if heard.is_empty() || sung.is_empty() {
        return None;
    }
    // edit-distance table over words, then walk back for the matched pairs
    let (n, m) = (sung.len(), heard.len());
    let mut d = vec![vec![0u32; m + 1]; n + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i as u32;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j as u32;
    }
    for i in 1..=n {
        for j in 1..=m {
            let same = u32::from(sung[i - 1].0 != heard[j - 1].0);
            d[i][j] = (d[i - 1][j - 1] + same).min(d[i - 1][j] + 1).min(d[i][j - 1] + 1);
        }
    }
    let mut line_time: Vec<Option<u64>> = vec![None; lines.len()];
    let (mut i, mut j) = (n, m);
    while i > 0 && j > 0 {
        if sung[i - 1].0 == heard[j - 1].0 && d[i][j] == d[i - 1][j - 1] {
            let li = sung[i - 1].1;
            // walking backwards: the last assignment is the line's earliest matched word
            line_time[li] = Some(heard[j - 1].1);
            i -= 1;
            j -= 1;
        } else if d[i][j] == d[i - 1][j] + 1 {
            i -= 1;
        } else {
            j -= 1;
        }
    }
    let text_lines: Vec<usize> = (0..lines.len()).filter(|&k| !lines[k].trim().is_empty()).collect();
    let matched = text_lines.iter().filter(|&&k| line_time[k].is_some()).count();
    if text_lines.is_empty() || matched * 2 < text_lines.len() {
        return None;
    }
    // fill the gaps between known lines, keep times in order
    let known: Vec<(usize, u64)> = text_lines.iter().filter_map(|&k| line_time[k].map(|t| (k, t))).collect();
    let mut out = Vec::new();
    let mut last = 0u64;
    for (k, line) in lines.iter().enumerate() {
        let t = line_time[k].unwrap_or_else(|| {
            let before = known.iter().rev().find(|(kk, _)| *kk < k);
            let after = known.iter().find(|(kk, _)| *kk > k);
            match (before, after) {
                (Some(&(bk, bt)), Some(&(ak, at))) => bt + (at.saturating_sub(bt)) * (k - bk) as u64 / (ak - bk) as u64,
                (Some(&(_, bt)), None) => bt + 2000,
                (None, Some(&(_, at))) => at.saturating_sub(2000),
                (None, None) => 0,
            }
        });
        let t = t.max(last);
        last = t;
        out.push(format!("{}{}", stamp(t, '[', ']'), line.trim()));
    }
    Some(out.join("\n"))
}

fn stamp(ms: u64, open: char, close: char) -> String {
    format!("{open}{:02}:{:02}.{:02}{close}", ms / 60_000, (ms / 1000) % 60, (ms % 1000) / 10)
}

/// Whisper writes these over music and silence; they are not lyrics.
fn noise(text: &str) -> bool {
    let t = text.trim();
    let lower = t.to_lowercase();
    t.is_empty()
        || t.chars().all(|c| !c.is_alphanumeric())
        || (t.starts_with('[') && t.ends_with(']'))
        || (t.starts_with('(') && t.ends_with(')'))
        || ["thank you for watching", "thanks for watching", "subtitles by", "please subscribe", "transcribed by"].iter().any(|n| lower.contains(n))
}

fn temp_file(ext: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!("lyrixearch-transcribe-{}-{n}.{ext}", std::process::id()))
}

/// `ffmpeg` turning the track into what the engine reads.
fn convert(input: &Path, output: &Path, args: &[&str]) -> Result<()> {
    let ffmpeg = find_tool("ffmpeg").ok_or_else(|| anyhow!("ffmpeg is needed to prepare the audio (install ffmpeg)"))?;
    let out = Command::new(ffmpeg).args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(input).args(args).arg(output).output()?;
    if !out.status.success() {
        bail!("ffmpeg could not read the track: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// whisper.cpp: WAV in, JSON with one entry per phrase out. Phrases are split at word boundaries
/// near 50 characters, which reads like lyric lines.
/// Settings from the research on Whisper and songs: beam search, no conditioning on the previous
/// text (the main cause of loops), non-speech tokens suppressed, and the names as a prompt.
fn local(path: &Path, cfg: &Config, hint: Option<&str>) -> Result<Vec<Segment>> {
    let bin = whisper_bin(cfg).ok_or_else(|| anyhow!("whisper.cpp (whisper-cli) is not installed: Settings → Transcription"))?;
    let model = model_path(cfg).ok_or_else(|| anyhow!("no Whisper model yet: download one in Settings → Transcription"))?;
    let wav = temp_file("wav");
    // whisper.cpp adds ".json" to the name it is given
    let base = temp_file("out");
    let json = PathBuf::from(format!("{}.json", base.display()));
    let cleanup = || {
        let _ = std::fs::remove_file(&wav);
        let _ = std::fs::remove_file(&json);
    };
    let result = (|| {
        convert(path, &wav, &["-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le"])?;
        let lang = cfg.transcribe_language.as_deref().filter(|l| !l.trim().is_empty()).unwrap_or("auto");
        let out = Command::new(&bin)
            .arg("-m")
            .arg(&model)
            .arg("-f")
            .arg(&wav)
            .args(["-l", lang, "-oj", "-np", "-ml", "50", "-sow", "-bs", "5", "-mc", "0", "-sns"])
            .args(hint.map(|h| vec!["--prompt".to_string(), h.to_string()]).unwrap_or_default())
            .arg("-of")
            .arg(&base)
            .output()?;
        if !out.status.success() {
            bail!("whisper.cpp failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("").trim());
        }
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&json).context("whisper.cpp wrote no result")?)?;
        Ok(v.get("transcription")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|s| Segment {
                        ms: s.pointer("/offsets/from").and_then(Value::as_u64).unwrap_or(0),
                        text: s.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                        words: Vec::new(),
                    })
                    .collect()
            })
            .unwrap_or_default())
    })();
    cleanup();
    result
}

/// An OpenAI-style transcription API (Groq, OpenAI) with the person's key; phrases and words timed.
async fn cloud(http: &reqwest::Client, path: &Path, cfg: &Config, hint: Option<&str>, url: &str, model: &str) -> Result<Vec<Segment>> {
    let key = cfg
        .transcribe_api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| anyhow!("no API key for transcription: Settings → Transcription"))?;
    // too big, or a format the API does not take: a small mono MP3 does
    let accepted = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["mp3", "m4a", "mp4", "flac", "ogg", "oga", "opus", "wav", "webm"].contains(&e.to_lowercase().as_str()));
    let size = std::fs::metadata(path)?.len();
    let (upload, temp) = if accepted && size <= UPLOAD_LIMIT {
        (path.to_path_buf(), None)
    } else {
        let mp3 = temp_file("mp3");
        let (p, m) = (path.to_path_buf(), mp3.clone());
        tokio::task::spawn_blocking(move || convert(&p, &m, &["-ac", "1", "-b:a", "64k"])).await??;
        (mp3.clone(), Some(mp3))
    };
    let bytes = std::fs::read(&upload);
    if let Some(t) = &temp {
        let _ = std::fs::remove_file(t);
    }
    let name = upload.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "audio.mp3".into());
    let mut form = reqwest::multipart::Form::new()
        .part("file", reqwest::multipart::Part::bytes(bytes?).file_name(name))
        .text("model", model.to_string())
        .text("response_format", "verbose_json")
        .text("timestamp_granularities[]", "segment")
        .text("timestamp_granularities[]", "word");
    if let Some(lang) = cfg.transcribe_language.as_deref().map(str::trim).filter(|l| !l.is_empty() && *l != "auto") {
        form = form.text("language", lang.to_string());
    }
    if let Some(h) = hint {
        form = form.text("prompt", h.to_string());
    }
    let resp = http.post(url).bearer_auth(key).multipart(form).timeout(Duration::from_secs(300)).send().await?;
    let status = resp.status();
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    if status.as_u16() == 429 {
        return Err(super::RateLimited { retry_after: Duration::from_secs(60), reason: "the transcription service asked to slow down".into() }.into());
    }
    if !status.is_success() {
        let msg = body.pointer("/error/message").and_then(Value::as_str).unwrap_or("");
        bail!("transcription failed (HTTP {status}){}", if msg.is_empty() { String::new() } else { format!(": {msg}") });
    }
    Ok(segments_from_api(&body))
}

/// `verbose_json`: `segments` (`start` seconds, `text`) and, when asked for, `words` (`start`, `word`).
fn segments_from_api(body: &Value) -> Vec<Segment> {
    let secs = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).map(|s| (s * 1000.0).round().max(0.0) as u64);
    let words: Vec<(u64, u64, String)> = body
        .get("words")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|w| Some((secs(w, "start")?, secs(w, "end").unwrap_or(0), w.get("word")?.as_str()?.trim().to_string()))).collect())
        .unwrap_or_default();
    body.get("segments")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|s| {
                    let (from, to) = (secs(s, "start").unwrap_or(0), secs(s, "end").unwrap_or(u64::MAX));
                    Segment {
                        ms: from,
                        text: s.get("text").and_then(Value::as_str).unwrap_or("").trim().to_string(),
                        words: words.iter().filter(|(st, _, w)| *st >= from && *st < to && !w.is_empty()).map(|(st, _, w)| (*st, w.clone())).collect(),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_lyrics_are_timed_by_the_transcription() {
        // heard with mistakes ("singing" for "signing"); the real lines get the heard times
        let rows = vec![
            Segment { ms: 1000, text: "Walking down the empty road tonight".into(), words: Vec::new() },
            Segment { ms: 5000, text: "every light is fading out of sight".into(), words: Vec::new() },
            Segment { ms: 9000, text: "I keep on singing just to feel alive".into(), words: Vec::new() },
        ];
        let lyrics = "Walking down the empty road tonight\nEvery light is fading out of sight\n\nI keep on signing just to feel alive";
        let lrc = align(lyrics, &rows).unwrap();
        assert_eq!(lrc, "[00:01.00]Walking down the empty road tonight\n[00:05.00]Every light is fading out of sight\n[00:07.00]\n[00:09.00]I keep on signing just to feel alive");
        assert!(align("Completely different words here\nNothing alike at all", &rows).is_none(), "too little matched to trust");
    }

    #[test]
    fn loops_are_cut_but_a_chorus_stays() {
        let seg = |t: &str| Segment { ms: 0, text: t.into(), words: Vec::new() };
        let rows = vec![seg("Hold on"), seg("Hold on"), seg("Hold on"), seg("Hold on"), seg("Hold on"), seg("We will survive")];
        assert_eq!(drop_loops(rows).iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Hold on", "Hold on", "Hold on", "We will survive"]);
    }

    #[test]
    fn api_segments_get_their_words() {
        let body = json!({
            "segments": [{ "start": 1.0, "end": 3.0, "text": " Hello there " }, { "start": 3.5, "end": 5.0, "text": "[Music]" }],
            "words": [{ "word": "Hello", "start": 1.0, "end": 1.4 }, { "word": "there", "start": 1.5, "end": 2.0 }]
        });
        let s = segments_from_api(&body);
        assert_eq!(s[0].ms, 1000);
        assert_eq!(s[0].text, "Hello there");
        assert_eq!(s[0].words, vec![(1000, "Hello".to_string()), (1500, "there".to_string())]);
        assert!(noise(&s[1].text), "[Music] is not a lyric");
        assert!(noise("♪ ♪") && noise("Thanks for watching!") && !noise("Thanks for the memories"));
    }
}
