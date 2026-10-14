//! External plugins: any executable that speaks a small JSON protocol over stdin/stdout.
//!
//! Layout: `<config>/lyrixearch/plugins/<name>/plugin.toml`
//!
//! ```toml
//! id = "metallum"                      # used in `providers = [...]`
//! name = "Metal Archives"
//! command = ["python3", "plugin.py"]   # runs with the plugin folder as working directory
//! timeout_secs = 30                    # optional, default 30
//! max_parallel = 1                     # optional, default 2: lookups running at once
//! min_gap_ms = 300                     # optional, default 300: pause between starting two lookups
//! ```
//!
//! Protocol v1 — the plugin reads one JSON object from stdin:
//! `{"protocol":1,"artist","title","album","album_artist","track_number","duration_secs","path"}`
//! and prints one JSON object on stdout:
//! `{"found":true,"plain":"…","synced":"[00:01.00]…","artist":"…","title":"…","url":"…","score":0.9}`
//! or `{"found":false}`. A non-zero exit status (message on stderr) is reported as a provider
//! failure and counts toward the provider's penalty.

use super::Provider;
use crate::config::Config;
use crate::lrc;
use crate::model::{Lyrics, Query};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub id: String,
    #[serde(default)]
    pub name: String,
    pub command: Vec<String>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// How many lookups may run at once (default 2). Use 1 for a script that talks to a slow or strict service.
    #[serde(default)]
    pub max_parallel: Option<usize>,
    /// Minimum pause between starting two lookups, in milliseconds.
    #[serde(default)]
    pub min_gap_ms: Option<u64>,
}

fn default_timeout() -> u64 {
    30
}

pub struct ExternalProvider {
    manifest: Manifest,
    dir: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Reply {
    #[serde(default)]
    found: bool,
    plain: Option<String>,
    synced: Option<String>,
    artist: Option<String>,
    title: Option<String>,
    url: Option<String>,
    score: Option<f32>,
}

impl ExternalProvider {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("plugin.toml");
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let manifest: Manifest = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if manifest.id.trim().is_empty() || manifest.command.is_empty() {
            bail!("{}: `id` and `command` are required", path.display());
        }
        Ok(Self { manifest, dir: dir.to_path_buf() })
    }

    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

/// Load every valid plugin under `dir`. Broken ones are returned as errors, not hidden.
pub fn discover(dir: &Path) -> (Vec<ExternalProvider>, Vec<(PathBuf, anyhow::Error)>) {
    let mut ok = Vec::new();
    let mut bad = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return (ok, bad) };
    let mut entries: Vec<_> = rd.filter_map(Result::ok).filter(|e| e.path().join("plugin.toml").is_file()).collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        match ExternalProvider::load(&e.path()) {
            Ok(p) => ok.push(p),
            Err(err) => bad.push((e.path(), err)),
        }
    }
    (ok, bad)
}

#[async_trait]
impl Provider for ExternalProvider {
    fn id(&self) -> &str {
        &self.manifest.id
    }

    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(self.manifest.timeout_secs.max(1)) + Duration::from_secs(5))
    }

    fn politeness(&self) -> Option<(usize, Duration)> {
        Some((self.manifest.max_parallel.unwrap_or(2).max(1), Duration::from_millis(self.manifest.min_gap_ms.unwrap_or(300))))
    }

    fn timeouts_count(&self) -> bool {
        false
    }

    async fn fetch(&self, _http: &reqwest::Client, q: &Query, _cfg: &Config) -> Result<Option<Lyrics>> {
        let payload = serde_json::json!({
            "protocol": 1,
            "artist": q.artist,
            "title": q.title,
            "album": q.album,
            "album_artist": q.album_artist,
            "track_number": q.track_number,
            "duration_secs": q.duration_secs,
            "path": q.source_path,
        });

        let (program, args) = self.manifest.command.split_first().expect("validated non-empty");
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("starting plugin `{}` ({program})", self.manifest.id))?;

        let mut stdin = child.stdin.take().context("plugin stdin")?;
        let bytes = serde_json::to_vec(&payload)?;
        // A plugin that exits without reading stdin must not turn into a broken-pipe failure.
        let _ = stdin.write_all(&bytes).await;
        drop(stdin);

        let out = tokio::time::timeout(Duration::from_secs(self.manifest.timeout_secs.max(1)), child.wait_with_output())
            .await
            .map_err(|_| anyhow::anyhow!("plugin `{}` timed out after {}s", self.manifest.id, self.manifest.timeout_secs))??;

        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("plugin `{}` failed ({}): {}", self.manifest.id, out.status, err.trim().lines().last().unwrap_or(""));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let text = stdout.trim();
        if text.is_empty() {
            return Ok(None);
        }
        // Tolerate plugins that print log lines before the JSON object: use the last line that parses.
        let reply: Reply = text
            .lines()
            .rev()
            .find_map(|l| serde_json::from_str(l.trim()).ok())
            .or_else(|| serde_json::from_str(text).ok())
            .with_context(|| format!("plugin `{}` printed no valid JSON reply", self.manifest.id))?;

        if !reply.found {
            return Ok(None);
        }
        let synced = reply.synced.filter(|s| lrc::looks_synced(s));
        let plain = reply.plain.filter(|s| !s.trim().is_empty());
        if plain.is_none() && synced.is_none() {
            return Ok(None);
        }
        Ok(Some(Lyrics {
            plain,
            synced,
            source: self.manifest.id.clone(),
            url: reply.url,
            matched_artist: reply.artist.unwrap_or_else(|| q.artist.clone()),
            matched_title: reply.title.unwrap_or_else(|| q.title.clone()),
            score: reply.score.unwrap_or(0.9).clamp(0.0, 1.0),
        }))
    }
}
