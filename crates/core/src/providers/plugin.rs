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
//!
//! Optional, for problems on the plugin's side (old plugins simply never send them):
//! `{"found":false,"error":"…"}` is a failure (counts toward the penalty) and
//! `{"found":false,"error":"…","retry_after":120}` means "rate limited, leave me alone for that
//! many seconds" (held between 15 and 900, like HTTP `Retry-After`).

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
    /// Run once in the installed folder when the plugin is installed, e.g. to compile it
    /// (`["nim", "c", "-d:release", "main.nim"]`). The install fails if it fails.
    #[serde(default)]
    pub build: Option<Vec<String>>,
    /// Shown when the build command's program is missing, e.g. "needs Nim: https://nim-lang.org".
    #[serde(default)]
    pub build_hint: Option<String>,
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
    /// The plugin's own failure (temporary or not).
    error: Option<String>,
    /// Seconds the plugin's service asked us to wait.
    retry_after: Option<u64>,
}

/// Turn a plugin's stdout into a result. Log lines before the JSON object are tolerated: the
/// last line that parses wins.
fn parse_reply(id: &str, stdout: &str, q: &Query) -> Result<Option<Lyrics>> {
    let text = stdout.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let reply: Reply = text
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str(l.trim()).ok())
        .or_else(|| serde_json::from_str(text).ok())
        .with_context(|| format!("plugin `{id}` printed no valid JSON reply"))?;

    if let Some(secs) = reply.retry_after {
        let reason = reply.error.unwrap_or_else(|| "asked to wait".into());
        return Err(super::RateLimited { retry_after: Duration::from_secs(secs.clamp(15, 900)), reason: format!("plugin `{id}`: {reason}") }.into());
    }
    if let Some(err) = reply.error.filter(|e| !e.trim().is_empty()) {
        bail!("plugin `{id}`: {err}");
    }
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
        source: id.to_string(),
        url: reply.url,
        matched_artist: reply.artist.unwrap_or_else(|| q.artist.clone()),
        matched_title: reply.title.unwrap_or_else(|| q.title.clone()),
        score: reply.score.unwrap_or(0.9).clamp(0.0, 1.0),
    }))
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

    /// Copy a plugin folder into `plugins_dir`, run its `build` command there if it has one, and
    /// load the result. A failed build leaves nothing half-installed behind.
    pub fn install(src: &Path, plugins_dir: &Path) -> Result<Self> {
        if !src.join("plugin.toml").is_file() {
            bail!("{} has no plugin.toml", src.display());
        }
        let staged = Self::load(src)?;
        let name = src.file_name().with_context(|| format!("bad plugin folder name: {}", src.display()))?;
        let dest = plugins_dir.join(name);
        std::fs::create_dir_all(plugins_dir).with_context(|| format!("creating {}", plugins_dir.display()))?;
        crate::util::copy_dir(src, &dest)?;
        if let Some(cmd) = staged.manifest.build.as_ref().filter(|c| !c.is_empty()) {
            let out = std::process::Command::new(&cmd[0]).args(&cmd[1..]).current_dir(&dest).output();
            let failure = match out {
                Ok(o) if o.status.success() => None,
                Ok(o) => {
                    let err = String::from_utf8_lossy(&o.stderr);
                    let tail: Vec<&str> = err.lines().rev().take(12).collect::<Vec<_>>().into_iter().rev().collect();
                    Some(format!("build failed ({}):\n{}", o.status, tail.join("\n")))
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    Some(format!("`{}` was not found{}", cmd[0], staged.manifest.build_hint.as_deref().map(|h| format!(": {h}")).unwrap_or_default()))
                }
                Err(e) => Some(format!("could not run `{}`: {e}", cmd[0])),
            };
            if let Some(why) = failure {
                let _ = std::fs::remove_dir_all(&dest);
                bail!("installing {}: {why}", staged.manifest.id);
            }
        }
        Self::load(&dest)
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
        parse_reply(&self.manifest.id, &String::from_utf8_lossy(&out.stdout), q)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Query {
        Query { artist: "Band".into(), title: "Song".into(), ..Default::default() }
    }

    #[test]
    fn found_reply_with_log_lines_before_it() {
        let out = "starting up\n{\"found\":true,\"plain\":\"line one\\nline two\",\"title\":\"Song (Live)\",\"score\":1.7}\n";
        let l = parse_reply("p", out, &q()).unwrap().unwrap();
        assert_eq!(l.plain.as_deref(), Some("line one\nline two"));
        assert_eq!(l.matched_title, "Song (Live)");
        assert_eq!(l.matched_artist, "Band", "missing names fall back to the query");
        assert_eq!(l.score, 1.0, "score is clamped");
        assert_eq!(l.source, "p");
    }

    #[test]
    fn misses_and_empty_answers_are_none() {
        assert!(parse_reply("p", "{\"found\":false}", &q()).unwrap().is_none());
        assert!(parse_reply("p", "", &q()).unwrap().is_none());
        assert!(parse_reply("p", "{\"found\":true,\"plain\":\"  \"}", &q()).unwrap().is_none());
        // "synced" that is not LRC is ignored
        assert!(parse_reply("p", "{\"found\":true,\"synced\":\"no stamps\"}", &q()).unwrap().is_none());
    }

    #[test]
    fn errors_and_rate_limits_are_failures() {
        let e = parse_reply("p", "{\"found\":false,\"error\":\"server down\"}", &q()).unwrap_err();
        assert!(e.to_string().contains("server down"));
        assert!(e.downcast_ref::<super::super::RateLimited>().is_none());

        let e = parse_reply("p", "{\"found\":false,\"error\":\"slow down\",\"retry_after\":3}", &q()).unwrap_err();
        let rl = e.downcast_ref::<super::super::RateLimited>().expect("rate limited");
        assert_eq!(rl.retry_after.as_secs(), 15, "held to at least 15 s");

        assert!(parse_reply("p", "not json at all", &q()).is_err());
    }

    /// A real process: the payload arrives on stdin, a non-zero exit is a failure, and a plugin
    /// that hangs is stopped at its timeout.
    #[cfg(unix)]
    #[tokio::test]
    async fn runs_a_real_plugin_process() {
        let dir = std::env::temp_dir().join(format!("lyrixearch-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let write = |script: &str, timeout: u64| {
            std::fs::write(dir.join("p.sh"), script).unwrap();
            std::fs::write(dir.join("plugin.toml"), format!("id = \"echo\"\ncommand = [\"/bin/sh\", \"p.sh\"]\ntimeout_secs = {timeout}\n")).unwrap();
            ExternalProvider::load(&dir).unwrap()
        };
        let http = reqwest::Client::new();
        let cfg = Config::default();

        // echoes the title it was given back as the lyric
        let p = write("read line; t=$(printf '%s' \"$line\" | sed 's/.*\"title\":\"\\([^\"]*\\)\".*/\\1/'); printf '{\"found\":true,\"plain\":\"%s first\\\\n%s second\"}\\n' \"$t\" \"$t\"\n", 5);
        let l = p.fetch(&http, &q(), &cfg).await.unwrap().unwrap();
        assert_eq!(l.plain.as_deref(), Some("Song first\nSong second"));

        let p = write("echo 'it broke' >&2; exit 3\n", 5);
        let e = p.fetch(&http, &q(), &cfg).await.unwrap_err();
        assert!(e.to_string().contains("it broke"), "{e}");

        let p = write("sleep 5\n", 1);
        let t = std::time::Instant::now();
        let e = p.fetch(&http, &q(), &cfg).await.unwrap_err();
        assert!(e.to_string().contains("timed out"), "{e}");
        assert!(t.elapsed() < Duration::from_secs(4));

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
