//! Orchestration: several tracks in flight, providers tried in order per track, slow or
//! failing providers skipped for a while.

use crate::clean::{clean_title, is_various_artists, strip_featuring, strip_trailing_brackets, unsuitable};
use crate::config::Config;
use crate::lastfm;
use crate::model::{Lyrics, Query, Track};
use crate::output;
use crate::providers::{self, Provider};
use crate::tags;
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet, VecDeque};
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
    /// `cfg.tag_clean_pattern`, compiled once.
    extra_clean: Option<regex::Regex>,
    /// Last.fm corrections already asked for, keyed by lower-cased `(artist, title)`.
    corrections: Mutex<HashMap<ArtistTitle, Option<ArtistTitle>>>,
}

/// An `(artist, title)` pair.
type ArtistTitle = (String, String);

/// Sources that only look at files beside the track, so they still run for a track whose names
/// are placeholders.
const NAME_FREE_SOURCES: &[&str] = &["local_files"];

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
        Self::with_providers(cfg, ordered, jar)
    }

    /// Build an engine around an explicit, ordered list of sources (`cfg.providers` is not consulted).
    /// Front ends use [`Engine::new`]; this is for tests and embedding.
    pub fn with_providers(cfg: Config, ordered: Vec<Arc<dyn Provider>>, jar: Arc<reqwest::cookie::Jar>) -> Result<Arc<Self>> {
        cfg.validate()?;
        let extra_clean = cfg.tag_clean_pattern.as_deref().filter(|s| !s.trim().is_empty()).map(regex::Regex::new).transpose()?;
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
            extra_clean,
            corrections: Mutex::new(HashMap::new()),
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

    /// Query variants for one provider, most specific first: the cleaned names, the album artist
    /// for a "Various Artists" track, the title without any trailing brackets, then the raw title.
    /// Duplicates are dropped.
    fn variants(&self, q: &Query) -> Vec<Query> {
        let base = strip_featuring(&q.title);
        let mut title = if self.cfg.tag_clean_enabled { clean_title(&base) } else { base.trim().to_string() };
        if let Some(rx) = &self.extra_clean {
            let t = rx.replace_all(&title, "").trim().to_string();
            if !t.is_empty() {
                title = t;
            }
        }
        let cleaned = Query { title, artist: strip_featuring(&q.artist), ..q.clone() };
        let mut out = vec![cleaned.clone()];
        if is_various_artists(&q.artist) {
            if let Some(aa) = q.album_artist.as_deref().filter(|a| !is_various_artists(a)) {
                out.push(Query { artist: aa.to_string(), ..cleaned.clone() });
            }
        }
        let bare = strip_trailing_brackets(&cleaned.title);
        if !bare.is_empty() {
            out.push(Query { title: bare, ..cleaned.clone() });
        }
        out.push(Query { title: q.title.clone(), ..cleaned });
        dedupe(out)
    }

    /// [`Engine::variants`], preceded by the Last.fm-corrected names when a key is set.
    async fn search_variants(&self, q: &Query, notes: &mut Vec<String>) -> Vec<Query> {
        let mut out = Vec::new();
        if let Some(key) = self.cfg.lastfm_api_key.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
            match self.lastfm_correction(key, q).await {
                Ok(Some((artist, title))) => {
                    notes.push(format!("last.fm: corrected to {artist} - {title}"));
                    out.extend(self.variants(&Query { artist, title, ..q.clone() }));
                }
                Ok(None) => {}
                Err(e) => notes.push(format!("last.fm: {e:#}")),
            }
        }
        out.extend(self.variants(q));
        dedupe(out)
    }

    async fn lastfm_correction(&self, key: &str, q: &Query) -> Result<Option<(String, String)>> {
        let cache_key = (q.artist.to_lowercase(), q.title.to_lowercase());
        if let Some(hit) = self.corrections.lock().unwrap().get(&cache_key) {
            return Ok(hit.clone());
        }
        let wait = Duration::from_secs(self.cfg.provider_timeout_secs.max(1));
        let found = tokio::time::timeout(wait, lastfm::correct(&self.http, key, &q.artist, &q.title)).await.context("timed out")??;
        self.corrections.lock().unwrap().insert(cache_key, found.clone());
        Ok(found)
    }

    /// Try providers in order for one track; first usable result wins.
    ///
    /// Each provider gets one request budget for the whole track (all its query variants together);
    /// time spent waiting for its turn under the rate limiter does not count against it.
    pub async fn find(&self, q: &Query, mut on_event: impl FnMut(&str, Option<&str>)) -> Outcome {
        let mut notes = Vec::new();
        let mut answered = 0usize;
        let not_worth_it = unsuitable(&q.artist, &q.title);
        let variants = if not_worth_it.is_some() { self.variants(q) } else { self.search_variants(q, &mut notes).await };
        for p in &self.providers {
            let id = p.id();
            if let Some(why) = not_worth_it.filter(|_| !NAME_FREE_SOURCES.contains(&id)) {
                let why = format!("not searched: {why}");
                on_event(id, Some(&why));
                notes.push(format!("{id}: {why}"));
                // a deliberate skip is an answer ("nothing to search for"), not a failure
                answered += 1;
                continue;
            }
            if let Some(left) = self.penalty_remaining(id) {
                let why = format!("penalised for another {}s", left.as_secs());
                on_event(id, Some(&why));
                notes.push(format!("{id}: {why}"));
                continue;
            }
            on_event(id, None);
            let budget = p.timeout().unwrap_or_else(|| Duration::from_secs(self.cfg.provider_timeout_secs.max(1)) * 2);
            let mut spent = Duration::ZERO;
            let mut failed = false;
            let mut only_unsynced = false;
            for v in &variants {
                // our turn at this source: the permit is held while the request runs, outside the budget
                let _turn = match self.limiters.get(id) {
                    Some(l) => l.acquire().await,
                    None => None,
                };
                let Some(left) = budget.checked_sub(spent).filter(|d| !d.is_zero()) else {
                    notes.push(format!("{id}: timed out after {}s", budget.as_secs()));
                    failed = p.timeouts_count();
                    break;
                };
                let started = Instant::now();
                let result = tokio::time::timeout(left, p.fetch(&self.http, v, &self.cfg)).await;
                spent += started.elapsed();
                match result {
                    Ok(Ok(Some(mut l))) if l.has_text() => {
                        if self.cfg.synced_only && !l.synced.as_deref().is_some_and(crate::lrc::looks_synced) {
                            only_unsynced = true;
                            continue;
                        }
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
                notes.push(if only_unsynced { format!("{id}: only unsynced lyrics (synced only is on)") } else { format!("{id}: no match") });
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

/// Keep the first of each case-insensitive `(artist, title)` pair, in order.
fn dedupe(v: Vec<Query>) -> Vec<Query> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|q| seen.insert((q.artist.to_lowercase(), q.title.to_lowercase()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::AtomicUsize;

    /// What a mock source does on each call.
    #[derive(Clone)]
    enum Act {
        Find { plain: Option<&'static str>, synced: Option<&'static str> },
        Miss,
        Fail,
        RateLimit(u64),
        Sleep(u64),
    }

    struct Mock {
        id: &'static str,
        act: Act,
        calls: AtomicUsize,
        seen: Mutex<Vec<(String, String)>>,
        budget: Option<Duration>,
    }

    impl Mock {
        fn new(id: &'static str, act: Act) -> Arc<Self> {
            Arc::new(Self { id, act, calls: AtomicUsize::new(0), seen: Mutex::new(Vec::new()), budget: None })
        }
        fn with_budget(id: &'static str, act: Act, ms: u64) -> Arc<Self> {
            Arc::new(Self { id, act, calls: AtomicUsize::new(0), seen: Mutex::new(Vec::new()), budget: Some(Duration::from_millis(ms)) })
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Provider for Mock {
        fn id(&self) -> &str {
            self.id
        }
        fn timeout(&self) -> Option<Duration> {
            self.budget
        }
        fn politeness(&self) -> Option<(usize, Duration)> {
            Some((8, Duration::ZERO))
        }
        async fn fetch(&self, _http: &reqwest::Client, q: &Query, _cfg: &Config) -> Result<Option<Lyrics>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.seen.lock().unwrap().push((q.artist.clone(), q.title.clone()));
            match self.act.clone() {
                Act::Find { plain, synced } => {
                    Ok(Some(Lyrics { plain: plain.map(String::from), synced: synced.map(String::from), source: self.id.into(), ..Default::default() }))
                }
                Act::Miss => Ok(None),
                Act::Fail => anyhow::bail!("boom"),
                Act::RateLimit(secs) => Err(providers::RateLimited { retry_after: Duration::from_secs(secs), reason: "slow down".into() }.into()),
                Act::Sleep(ms) => {
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    Ok(None)
                }
            }
        }
    }

    const WORDS: &str = "first line of the song\nsecond line of the song";
    const LRC: &str = "[00:01.00]first line of the song\n[00:02.00]second line of the song";

    fn engine(cfg: Config, ps: &[Arc<Mock>]) -> Arc<Engine> {
        let ps: Vec<Arc<dyn Provider>> = ps.iter().map(|p| p.clone() as Arc<dyn Provider>).collect();
        Engine::with_providers(cfg, ps, Arc::new(reqwest::cookie::Jar::default())).unwrap()
    }

    fn q(artist: &str, title: &str) -> Query {
        Query { artist: artist.into(), title: title.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn first_source_with_lyrics_wins_and_later_ones_are_not_asked() {
        let a = Mock::new("a", Act::Miss);
        let b = Mock::new("b", Act::Find { plain: Some(WORDS), synced: None });
        let c = Mock::new("c", Act::Find { plain: Some(WORDS), synced: None });
        let e = engine(Config::default(), &[a.clone(), b.clone(), c.clone()]);
        match e.find(&q("Queen", "Song"), |_, _| {}).await {
            Outcome::Found(l) => assert_eq!(l.source, "b"),
            other => panic!("expected Found, got {other:?}"),
        }
        assert!(a.calls() >= 1);
        assert_eq!(c.calls(), 0);
    }

    #[tokio::test]
    async fn every_source_failing_is_failed_but_a_clean_miss_is_not_found() {
        let e = engine(Config::default(), &[Mock::new("a", Act::Fail), Mock::new("b", Act::Fail)]);
        assert!(matches!(e.find(&q("Queen", "Song"), |_, _| {}).await, Outcome::Failed(_)));
        let e = engine(Config::default(), &[Mock::new("a", Act::Fail), Mock::new("b", Act::Miss)]);
        assert!(matches!(e.find(&q("Queen", "Song"), |_, _| {}).await, Outcome::NotFound(_)));
    }

    #[tokio::test]
    async fn repeated_failures_bench_a_source() {
        let cfg = Config { penalty_max_failures: 2, penalty_window_secs: 60, penalty_cooldown_secs: 300, ..Config::default() };
        let bad = Mock::new("bad", Act::Fail);
        let e = engine(cfg, std::slice::from_ref(&bad));
        for _ in 0..2 {
            e.find(&q("Queen", "Song"), |_, _| {}).await;
        }
        let calls = bad.calls();
        let mut skipped = Vec::new();
        let out = e.find(&q("Queen", "Song"), |id, why| skipped.push((id.to_string(), why.map(String::from)))).await;
        assert_eq!(bad.calls(), calls, "a benched source is not called");
        assert!(skipped[0].1.as_deref().unwrap().starts_with("penalised"));
        assert!(matches!(out, Outcome::Failed(_)));
        e.clear_penalty("bad");
        e.find(&q("Queen", "Song"), |_, _| {}).await;
        assert!(bad.calls() > calls, "clear_penalty lets it run again");
    }

    #[tokio::test]
    async fn rate_limited_source_cools_down_for_as_long_as_it_asked() {
        let rl = Mock::new("rl", Act::RateLimit(120));
        let e = engine(Config::default(), std::slice::from_ref(&rl));
        e.find(&q("Queen", "Song"), |_, _| {}).await;
        let left = e.penalty_remaining("rl").expect("cooling down");
        assert!(left > Duration::from_secs(100) && left <= Duration::from_secs(120));
    }

    #[tokio::test]
    async fn one_request_budget_covers_every_variant() {
        // Three variants (cleaned, bracket-less, raw). Each call takes 70 ms; the budget is 100 ms
        // for the whole track. With a budget per variant all three calls would fit (and the track
        // would end as NotFound); with one shared budget the second call runs out of time.
        // No wall-clock assertion: under load everything is slower, but never more calls.
        let slow = Mock::with_budget("slow", Act::Sleep(70), 100);
        let e = engine(Config::default(), std::slice::from_ref(&slow));
        let out = e.find(&q("Queen", "Song (Remastered 2011) [Demo]"), |_, _| {}).await;
        assert!(slow.calls() < 3, "made {} calls", slow.calls());
        assert!(matches!(out, Outcome::Failed(notes) if notes[0].contains("timed out")));
    }

    #[tokio::test]
    async fn variants_cover_cleaning_brackets_album_artist_and_raw() {
        let m = Mock::new("m", Act::Miss);
        let e = engine(Config::default(), std::slice::from_ref(&m));
        let mut query = q("Various Artists", "Song (Part 2) (Remastered 2011)");
        query.album_artist = Some("The Band".into());
        e.find(&query, |_, _| {}).await;
        let seen = m.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![
                ("Various Artists".into(), "Song (Part 2)".into()),
                ("The Band".into(), "Song (Part 2)".into()),
                ("Various Artists".into(), "Song".into()),
                ("Various Artists".into(), "Song (Part 2) (Remastered 2011)".into()),
            ]
        );
    }

    #[tokio::test]
    async fn title_clean_can_be_turned_off_or_extended() {
        let m = Mock::new("m", Act::Miss);
        let cfg = Config { tag_clean_enabled: false, ..Config::default() };
        engine(cfg, std::slice::from_ref(&m)).find(&q("A", "Song (Remastered)"), |_, _| {}).await;
        assert_eq!(m.seen.lock().unwrap()[0].1, "Song (Remastered)");

        let m = Mock::new("m", Act::Miss);
        let cfg = Config { tag_clean_pattern: Some(r"\s*-\s*demo$".into()), ..Config::default() };
        engine(cfg, std::slice::from_ref(&m)).find(&q("A", "Song - demo"), |_, _| {}).await;
        assert_eq!(m.seen.lock().unwrap()[0].1, "Song");

        let bad = Config { tag_clean_pattern: Some("(".into()), ..Config::default() };
        assert!(Engine::with_providers(bad, vec![], Arc::new(reqwest::cookie::Jar::default())).is_err());
    }

    #[tokio::test]
    async fn synced_only_skips_plain_results() {
        let plain = Mock::new("plain", Act::Find { plain: Some(WORDS), synced: None });
        let synced = Mock::new("synced", Act::Find { plain: None, synced: Some(LRC) });
        let cfg = Config { synced_only: true, ..Config::default() };
        match engine(cfg, &[plain.clone(), synced]).find(&q("Queen", "Song"), |_, _| {}).await {
            Outcome::Found(l) => {
                assert_eq!(l.source, "synced");
                assert_eq!(l.plain.as_deref(), Some(WORDS), "plain text is derived from the LRC");
            }
            other => panic!("expected Found, got {other:?}"),
        }
        let e = engine(Config { synced_only: true, ..Config::default() }, &[plain]);
        assert!(matches!(e.find(&q("Queen", "Song"), |_, _| {}).await, Outcome::NotFound(n) if n[0].contains("only unsynced")));
    }

    #[tokio::test]
    async fn placeholder_tracks_only_reach_local_sources() {
        let local = Mock::new("local_files", Act::Miss);
        let remote = Mock::new("remote", Act::Find { plain: Some(WORDS), synced: None });
        let e = engine(Config::default(), &[local.clone(), remote.clone()]);
        let out = e.find(&q("Unknown Artist", "Track 01"), |_, _| {}).await;
        assert_eq!(local.calls(), 1);
        assert_eq!(remote.calls(), 0);
        assert!(matches!(out, Outcome::NotFound(n) if n.iter().any(|x| x.contains("not searched"))));
    }

    #[tokio::test]
    async fn run_streams_one_final_event_per_track() {
        let e = engine(Config { concurrency: 2, ..Config::default() }, &[Mock::new("a", Act::Find { plain: Some(WORDS), synced: None })]);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let items = (0..5).map(|i| (i * 10, q("Queen", &format!("Song {i}")))).collect();
        e.run(items, tx, Arc::new(AtomicBool::new(false))).await;
        let mut found = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            if let Event::Found { idx, .. } = ev {
                found.push(idx);
            }
        }
        found.sort();
        assert_eq!(found, vec![0, 10, 20, 30, 40], "events carry the caller's ids");
    }
}
