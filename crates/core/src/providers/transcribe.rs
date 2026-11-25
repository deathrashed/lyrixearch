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
        let segments = match cfg.transcribe_engine.as_str() {
            "groq" => cloud(http, &path, cfg, "https://api.groq.com/openai/v1/audio/transcriptions", "whisper-large-v3-turbo").await?,
            "openai" => cloud(http, &path, cfg, "https://api.openai.com/v1/audio/transcriptions", "whisper-1").await?,
            _ => {
                let cfg = cfg.clone();
                tokio::task::spawn_blocking(move || local(&path, &cfg)).await.context("transcription stopped")??
            }
        };
        let rows: Vec<Segment> = segments.into_iter().filter(|s| !noise(&s.text)).collect();
        if rows.iter().map(|s| s.text.split_whitespace().count()).sum::<usize>() < 8 {
            // an instrumental, or nothing intelligible
            return Ok(None);
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
fn local(path: &Path, cfg: &Config) -> Result<Vec<Segment>> {
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
        let out = Command::new(&bin).arg("-m").arg(&model).arg("-f").arg(&wav).args(["-l", lang, "-oj", "-np", "-ml", "50", "-sow", "-of"]).arg(&base).output()?;
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
async fn cloud(http: &reqwest::Client, path: &Path, cfg: &Config, url: &str, model: &str) -> Result<Vec<Segment>> {
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
