//! Orchestration: several tracks in flight, providers tried in order per track, slow or
//! failing providers skipped for a while.

use crate::clean::{clean_title, is_various_artists, strip_featuring};
use crate::config::Config;
use crate::model::{Lyrics, Query, Track};
use crate::output;
use crate::providers::{self, Provider};
use crate::tags;
use anyhow::{Context, Result};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc::UnboundedSender, Semaphore};

/// Progress events, keyed by the caller's item index.
#[derive(Clone, Debug)]
pub enum Event {
    Trying {
        idx: usize,
        provider: String,
    },
    Skipped {
        idx: usize,
        provider: String,
        reason: String,
    },
    Found {
        idx: usize,
        lyrics: Box<Lyrics>,
    },
    NotFound {
        idx: usize,
        notes: Vec<String>,
    },
    Cancelled {
        idx: usize,
    },
    /// Every source errored or was benched, so nothing was learned about this track. Search again later.
    Failed {
        idx: usize,
        notes: Vec<String>,
    },
    /// A source needs a bot check passed (see [`providers::Challenge`]); announced once per site for a while.
    Challenge {
        provider: String,
        url: String,
    },
}

#[derive(Debug)]
pub enum Outcome {
    Found(Box<Lyrics>),
    NotFound(Vec<String>),
    /// No source gave an answer at all (every one errored or was benched): not the same as "no lyrics exist".
    Failed(Vec<String>),
}

#[derive(Default)]
struct Penalty {
    failures: VecDeque<Instant>,
    disabled_until: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub struct Applied {
    pub tag_written: bool,
    pub sidecar: Option<PathBuf>,
}

pub struct Engine {
    http: reqwest::Client,
    providers: Vec<Arc<dyn Provider>>,
    pub cfg: Config,
    penalties: Mutex<HashMap<String, Penalty>>,
    jar: Arc<reqwest::cookie::Jar>,
    limiters: HashMap<String, Arc<Limiter>>,
    challenges: Mutex<ChallengeBook>,
}

/// Caps how many requests one source gets at once and spaces their starts.
struct Limiter {
    slots: Arc<Semaphore>,
    gap: Duration,
    next: tokio::sync::Mutex<Instant>,
}

impl Limiter {
    fn new(p: &dyn Provider) -> Self {
        let (inflight, gap) = p.politeness().unwrap_or_else(|| providers::politeness(p.id()));
        Self { slots: Arc::new(Semaphore::new(inflight.max(1))), gap, next: tokio::sync::Mutex::new(Instant::now()) }
    }

    /// Wait for a slot and for our turn; hold the returned permit while the request runs.
    async fn acquire(&self) -> Option<tokio::sync::OwnedSemaphorePermit> {
        let permit = self.slots.clone().acquire_owned().await.ok()?;
        if !self.gap.is_zero() {
            let mut next = self.next.lock().await;
            let now = Instant::now();
            if *next > now {
                tokio::time::sleep(*next - now).await;
            }
            *next = Instant::now() + self.gap;
        }
        Some(permit)
    }
}

#[derive(Default)]
struct ChallengeBook {
    pending: Vec<(String, String)>,
    announced: HashMap<String, Instant>,
}

impl Engine {
    pub fn new(cfg: Config) -> Result<Arc<Self>> {
        Self::with_cookies(cfg, Arc::new(reqwest::cookie::Jar::default()))
    }

    /// Like [`Engine::new`], sharing a cookie jar so cookies from a passed bot check survive a rebuild.
    pub fn with_cookies(cfg: Config, jar: Arc<reqwest::cookie::Jar>) -> Result<Arc<Self>> {
        let known = providers::all(&Config::plugins_dir());
        let mut ordered = Vec::new();
        for id in &cfg.providers {
            let p = known
                .iter()
                .find(|p| p.id() == id)
                .with_context(|| format!("unknown provider `{id}` (known: {})", known.iter().map(|p| p.id()).collect::<Vec<_>>().join(", ")))?;
            ordered.push(p.clone());
        }
        let mut b = reqwest::Client::builder()
            .gzip(true)
            .cookie_provider(jar.clone())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(cfg.provider_timeout_secs.max(1)));
        if let Some(proxy) = &cfg.proxy {
            b = b.proxy(reqwest::Proxy::all(proxy).context("invalid proxy url")?);
        }
        let limiters = ordered.iter().map(|p| (p.id().to_string(), Arc::new(Limiter::new(p.as_ref())))).collect();
        Ok(Arc::new(Self {
            http: b.build()?,
            providers: ordered,
            cfg,
            penalties: Mutex::new(HashMap::new()),
            limiters,
            jar,
            challenges: Mutex::new(ChallengeBook::default()),
        }))
    }

    pub fn cookie_jar(&self) -> Arc<reqwest::cookie::Jar> {
        self.jar.clone()
    }

    /// Forget a source's recent failures after its bot check was passed.
    pub fn clear_penalty(&self, provider: &str) {
        self.penalties.lock().unwrap().remove(provider);
    }

    /// Bot checks seen since the last call, once per site every ten minutes.
    pub fn take_challenges(&self) -> Vec<(String, String)> {
        std::mem::take(&mut self.challenges.lock().unwrap().pending)
    }

    /// Allow the same site to be announced again (the window was closed, or the user asks to retry).
    pub fn forget_announced(&self) {
        self.challenges.lock().unwrap().announced.clear();
    }

    fn note_challenge(&self, provider: &str, url: &str) {
        let host = url.split('/').nth(2).unwrap_or(url).to_string();
        let mut book = self.challenges.lock().unwrap();
        let fresh = book.announced.get(&host).is_none_or(|t| t.elapsed() > Duration::from_secs(600));
        if fresh {
            book.announced.insert(host, Instant::now());
            book.pending.push((provider.to_string(), url.to_string()));
        }
    }

    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.iter().map(|p| p.id().to_string()).collect()
    }

    /// Leave a source alone for as long as it asked.
    fn cool_down(&self, id: &str, wait: Duration) {
        let mut map = self.penalties.lock().unwrap();
        let p = map.entry(id.to_string()).or_default();
        let until = Instant::now() + wait;
        if p.disabled_until.is_none_or(|t| t < until) {
            p.disabled_until = Some(until);
        }
    }

    fn penalty_remaining(&self, id: &str) -> Option<Duration> {
        let mut map = self.penalties.lock().unwrap();
        let p = map.entry(id.to_string()).or_default();
        match p.disabled_until {
            Some(t) if t > Instant::now() => Some(t - Instant::now()),
            Some(_) => {
                p.disabled_until = None;
                p.failures.clear();
                None
            }
            None => None,
        }
    }

    fn record_failure(&self, id: &str) {
        let mut map = self.penalties.lock().unwrap();
        let p = map.entry(id.to_string()).or_default();
        let now = Instant::now();
        let window = Duration::from_secs(self.cfg.penalty_window_secs);
        p.failures.push_back(now);
        while p.failures.front().is_some_and(|t| now.duration_since(*t) > window) {
            p.failures.pop_front();
        }
        if p.failures.len() >= self.cfg.penalty_max_failures.max(1) {
            p.disabled_until = Some(now + Duration::from_secs(self.cfg.penalty_cooldown_secs));
        }
    }

    fn record_success(&self, id: &str) {
        self.penalties.lock().unwrap().entry(id.to_string()).or_default().failures.clear();
    }

    /// Query variants for one provider: cleaned first, then album-artist and raw fallbacks.
    fn variants(q: &Query) -> Vec<Query> {
        let mut out = Vec::new();
        let cleaned = Query { title: clean_title(&strip_featuring(&q.title)), artist: strip_featuring(&q.artist), ..q.clone() };
        out.push(cleaned.clone());
        if is_various_artists(&q.artist) {
            if let Some(aa) = q.album_artist.as_deref().filter(|a| !is_various_artists(a)) {
                out.push(Query { artist: aa.to_string(), ..cleaned.clone() });
            }
        }
        if cleaned.title != q.title {
            out.push(Query { title: q.title.clone(), ..cleaned });
        }
        out
    }

    /// Try providers in order for one track; first usable result wins.
    pub async fn find(&self, q: &Query, mut on_event: impl FnMut(&str, Option<&str>)) -> Outcome {
        let mut notes = Vec::new();
        let mut answered = 0usize;
        let variants = Self::variants(q);
        for p in &self.providers {
            let id = p.id();
            if let Some(left) = self.penalty_remaining(id) {
                let why = format!("penalised for another {}s", left.as_secs());
                on_event(id, Some(&why));
                notes.push(format!("{id}: {why}"));
                continue;
            }
            on_event(id, None);
            let budget = p.timeout().unwrap_or_else(|| Duration::from_secs(self.cfg.provider_timeout_secs.max(1)) * 2);
            let mut failed = false;
            for v in &variants {
                // our turn at this source: the permit is held while the request runs, outside the timeout
                let _turn = match self.limiters.get(id) {
                    Some(l) => l.acquire().await,
                    None => None,
                };
                match tokio::time::timeout(budget, p.fetch(&self.http, v, &self.cfg)).await {
                    Ok(Ok(Some(mut l))) if l.has_text() => {
                        l.plain = l.embed_text();
                        self.record_success(id);
                        return Outcome::Found(Box::new(l));
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => {
                        if let Some(c) = e.downcast_ref::<providers::Challenge>() {
                            self.note_challenge(id, &c.url);
                        }
                        if let Some(r) = e.downcast_ref::<providers::RateLimited>() {
                            self.cool_down(id, r.retry_after);
                        }
                        notes.push(format!("{id}: {e:#}"));
                        failed = true;
                        break;
                    }
                    Err(_) => {
                        notes.push(format!("{id}: timed out after {}s", budget.as_secs()));
                        failed = p.timeouts_count();
                        break;
                    }
                }
            }
            if failed {
                self.record_failure(id);
            } else {
                self.record_success(id);
                notes.push(format!("{id}: no match"));
                answered += 1;
            }
        }
        if answered == 0 && !notes.is_empty() {
            return Outcome::Failed(notes);
        }
        Outcome::NotFound(notes)
    }

    /// Search many tracks with bounded concurrency, streaming progress through `tx`.
    /// Set `cancel` to stop starting new work; in-flight lookups finish.
    pub async fn run(self: &Arc<Self>, items: Vec<(usize, Query)>, tx: UnboundedSender<Event>, cancel: Arc<AtomicBool>) {
        let sem = Arc::new(Semaphore::new(self.cfg.concurrency.max(1)));
        let mut handles = Vec::new();
        for (idx, q) in items {
            let engine = self.clone();
            let tx = tx.clone();
            let sem = sem.clone();
            let cancel = cancel.clone();
            handles.push(tokio::spawn(async move {
                let Ok(_permit) = sem.acquire_owned().await else { return };
                if cancel.load(Ordering::Relaxed) {
                    let _ = tx.send(Event::Cancelled { idx });
                    return;
                }
                let txc = tx.clone();
                // `find` takes a sync callback, so collect events through the channel directly.
                let outcome = engine
                    .find(&q, |provider, skipped| {
                        let _ = match skipped {
                            Some(reason) => txc.send(Event::Skipped { idx, provider: provider.into(), reason: reason.into() }),
                            None => txc.send(Event::Trying { idx, provider: provider.into() }),
                        };
                    })
                    .await;
                for (provider, url) in engine.take_challenges() {
                    let _ = tx.send(Event::Challenge { provider, url });
                }
                let _ = match outcome {
                    Outcome::Found(l) => tx.send(Event::Found { idx, lyrics: l }),
                    Outcome::NotFound(notes) => tx.send(Event::NotFound { idx, notes }),
                    Outcome::Failed(notes) => tx.send(Event::Failed { idx, notes }),
                };
            }));
        }
        for h in handles {
            let _ = h.await;
        }
    }

    /// Write a found result to the file: tags and/or sidecar, per config. Blocking I/O.
    pub fn apply(&self, track: &Track, lyrics: &Lyrics) -> Result<Applied> {
        let mut applied = Applied::default();
        if self.cfg.write_tags {
            if let Some(text) = lyrics.embed_text() {
                let text = output::postprocess(&text, &self.cfg);
                let synced = lyrics.synced.as_deref().filter(|s| crate::lrc::looks_synced(s));
                tags::write_lyrics_with(&track.path, &text, synced, self.cfg.keep_mtime)?;
                applied.tag_written = true;
            }
        }
        applied.sidecar = output::write_sidecar(&track.path, lyrics, &self.cfg)?;
        Ok(applied)
    }

    /// Health-check every configured provider with a well-known query (the original's "test mode").
    pub async fn test_providers(&self) -> Vec<(String, std::result::Result<bool, String>, Duration)> {
        let q = Query { artist: "Queen".into(), title: "Bohemian Rhapsody".into(), duration_secs: Some(355), ..Default::default() };
        let mut out = Vec::new();
        for p in &self.providers {
            let t = Instant::now();
            let r = match tokio::time::timeout(Duration::from_secs(self.cfg.provider_timeout_secs.max(1) * 2), p.fetch(&self.http, &q, &self.cfg)).await {
                Ok(Ok(l)) => Ok(l.is_some_and(|l| l.has_text())),
                Ok(Err(e)) => Err(format!("{e:#}")),
                Err(_) => Err("timed out".into()),
            };
            out.push((p.id().to_string(), r, t.elapsed()));
        }
        out
    }
}
