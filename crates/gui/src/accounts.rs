//! Signed-in sessions for the account sources (Spotify, Apple Music, Deezer).
//!
//! Each service gets one window of its own, hidden while it works. The person signs in on the
//! service's own page in that window (the app never sees a password; the service keeps its usual
//! session cookies). A lookup then runs a small script in that page which uses the service's own
//! web client: the token Spotify's web player already holds, Apple's MusicKit, Deezer's site API.
//! The answers go back to `lyrixearch_core::providers::accounts`, which picks the track and turns
//! the lyrics into text and LRC.

use anyhow::{anyhow, bail, Result};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WindowEvent};

struct Service {
    id: &'static str,
    name: &'static str,
    /// Where the web player lives; lookups run on this page.
    home: &'static str,
    /// Where signing in starts.
    sign_in: &'static str,
    /// The service's operations (`status`, `search`, `lyrics`, `signout`), as the body of `OPS`.
    ops: &'static str,
}

const SERVICES: &[Service] = &[
    Service {
        id: "spotify",
        name: "Spotify",
        home: "https://open.spotify.com/",
        sign_in: "https://accounts.spotify.com/login?continue=https%3A%2F%2Fopen.spotify.com%2F",
        ops: SPOTIFY_OPS,
    },
    Service { id: "apple_music", name: "Apple Music", home: "https://music.apple.com/", sign_in: "https://music.apple.com/login", ops: APPLE_OPS },
    Service { id: "deezer", name: "Deezer", home: "https://www.deezer.com/", sign_in: "https://www.deezer.com/login", ops: DEEZER_OPS },
];

fn service(id: &str) -> Result<&'static Service> {
    SERVICES.iter().find(|s| s.id == id).ok_or_else(|| anyhow!("unknown account service `{id}`"))
}

/// Runs in every page of a service window (before the page's own scripts). `window.__lxCall` starts
/// an operation; its result lands in `window.__lxRes[id]` for the app to collect.
const PRELUDE: &str = r#"(() => {
  if (window.__lxReady) return;
  window.__lxReady = true;
  window.__lxRes = {};
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  // Spotify's and Apple's web players refuse to start where the browser has no DRM (WebKitGTK on
  // Linux has no Widevine), and then never sign in or fetch anything. Lyrics need no protected
  // content, so where the real check fails a stand-in answers it: the player starts, and anything
  // it might try to play simply fails. Browsers with working DRM keep their own answer.
  (() => {
    const real = navigator.requestMediaKeySystemAccess && navigator.requestMediaKeySystemAccess.bind(navigator);
    const standIn = (keySystem, configs) => ({
      keySystem,
      getConfiguration: () => (configs && configs[0]) || {},
      createMediaKeys: async () => ({ createSession: () => ({ addEventListener() {}, removeEventListener() {}, generateRequest: async () => { throw new Error('no DRM here'); }, close: async () => {}, closed: new Promise(() => {}), keyStatuses: new Map(), sessionId: '' }), setServerCertificate: async () => true }),
    });
    const ask = async (keySystem, configs) => {
      if (real) { try { return await real(keySystem, configs); } catch (e) { /* no DRM: answered below */ } }
      return standIn(keySystem, configs);
    };
    try { Object.defineProperty(Navigator.prototype, 'requestMediaKeySystemAccess', { configurable: true, value: function (k, c) { return ask(k, c); } }); } catch (e) { /* left as it is */ }
  })();
  let OPS = {};
  __OPS__
  window.__lxCall = (id, op, args) => {
    Promise.resolve().then(() => {
      if (!OPS[op]) throw new Error('unknown operation ' + op);
      return OPS[op](args || {});
    }).then(
      (value) => { window.__lxRes[id] = { ok: true, value }; },
      (e) => { window.__lxRes[id] = { ok: false, error: String((e && e.message) || e) }; },
    );
  };
})();"#;

/// Spotify: the web player's own access token, as its requests carry it, used for the Web API
/// search and the player's lyrics endpoint (the same one its lyrics view calls).
const SPOTIFY_OPS: &str = r#"
  let token = null;
  const remember = (h) => {
    try {
      const v = h && (typeof h.get === 'function' ? h.get('authorization') : (h.authorization || h.Authorization));
      if (v && /^Bearer\s/.test(v)) token = v;
    } catch (e) { /* not a header bag */ }
  };
  const ofetch = window.fetch;
  window.fetch = function (input, init) {
    remember(init && init.headers);
    if (input && input.headers) remember(input.headers);
    return ofetch.apply(this, arguments);
  };
  const oset = XMLHttpRequest.prototype.setRequestHeader;
  XMLHttpRequest.prototype.setRequestHeader = function (k, v) {
    if (/^authorization$/i.test(k) && /^Bearer\s/.test(v)) token = v;
    return oset.apply(this, arguments);
  };
  // the player shows either its log-in button or the account menu once it has started
  const signedOut = async () => {
    for (let i = 0; i < 40 && !document.querySelector('[data-testid="login-button"], [data-testid="user-widget-link"]'); i++) await sleep(250);
    return !!document.querySelector('[data-testid="login-button"]');
  };
  const auth = async () => {
    for (let i = 0; i < 60 && !token; i++) await sleep(250);
    if (!token) throw new Error('the Spotify web player has not started yet');
    return token;
  };
  const api = async (url) => {
    const r = await ofetch(url, { headers: { authorization: await auth(), 'app-platform': 'WebPlayer', accept: 'application/json' } });
    if (r.status === 401) throw new Error('not signed in to Spotify: Settings → Accounts');
    if (r.status === 404) return null;
    if (r.status === 429) throw new Error('Spotify asked to slow down');
    if (!r.ok) throw new Error('Spotify answered HTTP ' + r.status);
    return r.json();
  };
  OPS = {
    status: async () => {
      if (await signedOut()) return { signedIn: false };
      const me = await api('https://api.spotify.com/v1/me').catch(() => null);
      return { signedIn: !!me, name: me && (me.display_name || me.id) };
    },
    search: async ({ artist, title }) => {
      if (await signedOut()) throw new Error('not signed in to Spotify: Settings → Accounts');
      const r = await api('https://api.spotify.com/v1/search?type=track&limit=10&q=' + encodeURIComponent('track:' + title + ' artist:' + artist));
      const items = (r && r.tracks && r.tracks.items) || [];
      return { tracks: items.map((t) => ({ id: t.id, title: t.name, artist: (t.artists && t.artists[0] && t.artists[0].name) || '', album: (t.album && t.album.name) || '', duration_ms: t.duration_ms, url: t.external_urls && t.external_urls.spotify })) };
    },
    lyrics: async ({ id }) => {
      const r = await api('https://spclient.wg.spotify.com/color-lyrics/v2/track/' + id + '?format=json&vocalRemoval=false&market=from_token');
      if (!r || !r.lyrics) return {};
      return { synced: r.lyrics.syncType === 'LINE_SYNCED', lines: (r.lyrics.lines || []).map((l) => ({ ms: Number(l.startTimeMs), text: l.words })) };
    },
    signout: async () => { setTimeout(() => { location.href = 'https://www.spotify.com/logout/'; }, 0); return {}; },
  };
"#;

/// Apple Music: MusicKit, the web player's own client (needs an Apple Music subscription).
const APPLE_OPS: &str = r#"
  const kit = async () => {
    for (let i = 0; i < 80; i++) {
      const m = window.MusicKit && typeof window.MusicKit.getInstance === 'function' && window.MusicKit.getInstance();
      if (m) return m;
      await sleep(250);
    }
    throw new Error('Apple Music has not started yet');
  };
  const store = (m) => m.storefrontId || (m.storefrontCountryCode) || 'us';
  OPS = {
    status: async () => { const m = await kit().catch(() => null); return { signedIn: !!(m && m.isAuthorized), name: null }; },
    search: async ({ artist, title }) => {
      const m = await kit();
      if (!m.isAuthorized) throw new Error('not signed in to Apple Music: Settings → Accounts');
      const r = await m.api.music('/v1/catalog/' + store(m) + '/search', { term: artist + ' ' + title, types: 'songs', limit: 10 });
      const songs = (r && r.data && r.data.results && r.data.results.songs && r.data.results.songs.data) || [];
      return { tracks: songs.filter((s) => s.attributes && s.attributes.hasLyrics !== false).map((s) => ({ id: s.id, title: s.attributes.name, artist: s.attributes.artistName, album: s.attributes.albumName || '', duration_ms: s.attributes.durationInMillis, url: s.attributes.url })) };
    },
    lyrics: async ({ id }) => {
      const m = await kit();
      const get = async (kind) => {
        const r = await m.api.music('/v1/catalog/' + store(m) + '/songs/' + id + '/' + kind);
        const d = r && r.data && r.data.data && r.data.data[0];
        return d && d.attributes && d.attributes.ttml;
      };
      const ttml = (await get('syllable-lyrics').catch(() => null)) || (await get('lyrics').catch(() => null));
      return ttml ? { ttml } : {};
    },
    signout: async () => { const m = await kit().catch(() => null); if (m) await m.unauthorize(); return {}; },
  };
"#;

/// Deezer: the site's own API (gw-light for the session and search; the "pipe" GraphQL API, with
/// the session's short-lived token, for lyrics, falling back to the older song.getLyrics).
const DEEZER_OPS: &str = r#"
  const gw = async (method, body, token) => {
    const r = await fetch('/ajax/gw-light.php?method=' + method + '&input=3&api_version=1.0&api_token=' + (token || ''), { method: 'POST', credentials: 'include', body: JSON.stringify(body || {}) });
    return r.json();
  };
  const user = async () => {
    const r = await gw('deezer.getUserData', {});
    const u = (r && r.results) || {};
    const id = u.USER && u.USER.USER_ID;
    return { signedIn: !!id && String(id) !== '0', name: u.USER && u.USER.BLOG_NAME, token: u.checkForm };
  };
  OPS = {
    status: async () => { const u = await user(); return { signedIn: u.signedIn, name: u.name }; },
    search: async ({ artist, title }) => {
      const u = await user();
      if (!u.signedIn) throw new Error('not signed in to Deezer: Settings → Accounts');
      const r = await gw('deezer.pageSearch', { query: artist + ' ' + title, start: 0, nb: 10, suggest: false, artist_suggest: false, top_tracks: false }, u.token);
      const data = (r && r.results && r.results.TRACK && r.results.TRACK.data) || [];
      return { tracks: data.map((t) => ({ id: String(t.SNG_ID), title: t.SNG_TITLE + (t.VERSION ? ' ' + t.VERSION : ''), artist: t.ART_NAME, album: t.ALB_TITLE || '', duration_ms: Number(t.DURATION) * 1000, url: 'https://www.deezer.com/track/' + t.SNG_ID })) };
    },
    lyrics: async ({ id }) => {
      try {
        const a = await fetch('https://auth.deezer.com/login/renew?jo=p&rto=c&i=c', { method: 'POST', credentials: 'include' }).then((r) => r.json());
        if (a && a.jwt) {
          const query = 'query SynchronizedTrackLyrics($trackId: String!) { track(trackId: $trackId) { lyrics { text synchronizedLines { line milliseconds } } } }';
          const r = await fetch('https://pipe.deezer.com/api', { method: 'POST', headers: { 'content-type': 'application/json', authorization: 'Bearer ' + a.jwt }, body: JSON.stringify({ operationName: 'SynchronizedTrackLyrics', variables: { trackId: id }, query }) }).then((r) => r.json());
          const ly = r && r.data && r.data.track && r.data.track.lyrics;
          if (ly) {
            const lines = ly.synchronizedLines || [];
            if (lines.length) return { synced: true, lines: lines.map((l) => ({ ms: Number(l.milliseconds), text: l.line })) };
            if (ly.text) return { plain: ly.text };
          }
        }
      } catch (e) { /* the older call below */ }
      const u = await user();
      const r = await gw('song.getLyrics', { sng_id: id }, u.token);
      const res = r && r.results;
      if (!res) return {};
      if (res.LYRICS_SYNC_JSON && res.LYRICS_SYNC_JSON.length) return { synced: true, lines: res.LYRICS_SYNC_JSON.filter((l) => l.line !== undefined).map((l) => ({ ms: Number(l.milliseconds), text: l.line })) };
      return res.LYRICS_TEXT ? { plain: res.LYRICS_TEXT } : {};
    },
    signout: async () => { setTimeout(() => { location.href = 'https://www.deezer.com/logout.php'; }, 0); return {}; },
  };
"#;

/// A service's window closes after this long without a lookup. It is a whole web player (150 to
/// 300 MB); the session survives in the app's browser storage, so the next lookup only reloads it.
const IDLE: Duration = Duration::from_secs(120);

pub struct Accounts {
    app: AppHandle,
    /// One lookup at a time per service; different services run side by side.
    turns: Vec<tokio::sync::Mutex<()>>,
    last_used: std::sync::Mutex<std::collections::HashMap<&'static str, Instant>>,
    next: AtomicU64,
}

impl Accounts {
    pub fn new(app: AppHandle) -> Self {
        Self { app, turns: SERVICES.iter().map(|_| tokio::sync::Mutex::new(())).collect(), last_used: Default::default(), next: AtomicU64::new(1) }
    }

    /// Close service windows that have been idle for a while (not while one is shown for signing in,
    /// nor while a lookup runs).
    pub fn start_idle_close(this: std::sync::Arc<Self>) {
        tauri::async_runtime::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                for (i, s) in SERVICES.iter().enumerate() {
                    let Some(w) = this.app.get_webview_window(&format!("acct-{}", s.id)) else { continue };
                    let idle = this.last_used.lock().unwrap().get(s.id).is_none_or(|t| t.elapsed() > IDLE);
                    if !idle || w.is_visible().unwrap_or(true) {
                        continue;
                    }
                    if let Ok(_turn) = this.turns[i].try_lock() {
                        let _ = w.destroy();
                    }
                }
            }
        });
    }

    /// The service's window, created (hidden, on the web player) on first use.
    fn window(app: &AppHandle, s: &Service) -> Result<WebviewWindow> {
        let label = format!("acct-{}", s.id);
        if let Some(w) = app.get_webview_window(&label) {
            return Ok(w);
        }
        let w = tauri::WebviewWindowBuilder::new(app, &label, WebviewUrl::External(s.home.parse()?))
            .title(format!("Sign in to {}", s.name))
            .inner_size(480.0, 720.0)
            .center()
            .visible(false)
            .focused(false)
            .user_agent(crate::browser::BROWSER_UA)
            .initialization_script(PRELUDE.replace("__OPS__", s.ops))
            // signing in may open the service's own sign-in popup
            .on_new_window(|_, _| tauri::webview::NewWindowResponse::Allow)
            .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))?)?
            .build()?;
        // closing it only hides it (the session stays), and goes back to the web player
        let (hide, home, app2) = (w.clone(), s.home, app.clone());
        w.on_window_event(move |ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = hide.hide();
                if let Ok(u) = home.parse() {
                    let _ = hide.navigate(u);
                }
                let _ = app2.emit("accounts-changed", ());
            }
        });
        Ok(w)
    }

    /// Show the service's sign-in page.
    pub fn sign_in(app: &AppHandle, id: &str) -> Result<()> {
        let s = service(id)?;
        let w = Self::window(app, s)?;
        w.navigate(s.sign_in.parse()?)?;
        w.show()?;
        w.set_focus()?;
        Ok(())
    }

    /// Run `op` in the service's page; the page must be the web player, so it is put back there first.
    pub async fn run(&self, id: &str, op: &str, args: Value, limit: Duration) -> Result<Value> {
        let s = service(id)?;
        let index = SERVICES.iter().position(|x| x.id == s.id).unwrap_or(0);
        let _turn = self.turns[index].lock().await;
        self.last_used.lock().unwrap().insert(s.id, Instant::now());
        let w = Self::window(&self.app, s)?;
        let home_host = s.home.split('/').nth(2).unwrap_or("");
        let on_home = w.url().ok().is_some_and(|u| u.host_str() == Some(home_host));
        if !on_home && !w.is_visible().unwrap_or(false) {
            w.navigate(s.home.parse()?)?;
            tokio::time::sleep(Duration::from_millis(1500)).await;
        }
        let call = self.next.fetch_add(1, Ordering::Relaxed);
        let args = serde_json::to_string(&args)?;
        let js = format!(
            "(() => {{ if (!window.__lxCall) return null; window.__lxStarted = window.__lxStarted || {{}}; if (!window.__lxStarted[{call}]) {{ window.__lxStarted[{call}] = 1; window.__lxCall({call}, {op:?}, {args}); }} return window.__lxRes[{call}] || null; }})()"
        );
        let started = Instant::now();
        loop {
            let (tx, rx) = tokio::sync::oneshot::channel::<String>();
            let tx = std::sync::Mutex::new(Some(tx));
            w.eval_with_callback(js.clone(), move |json| {
                if let Some(tx) = tx.lock().unwrap().take() {
                    let _ = tx.send(json);
                }
            })?;
            if let Ok(Ok(json)) = tokio::time::timeout(Duration::from_secs(5), rx).await {
                if let Ok(v) = serde_json::from_str::<Value>(&json) {
                    if !v.is_null() {
                        if v.get("ok").and_then(Value::as_bool) == Some(true) {
                            return Ok(v.get("value").cloned().unwrap_or(Value::Null));
                        }
                        bail!("{}", v.get("error").and_then(Value::as_str).unwrap_or("the page reported an error"));
                    }
                }
            }
            if started.elapsed() > limit {
                bail!("{} did not answer in time", s.name);
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    }
}

#[async_trait::async_trait]
impl lyrixearch_core::providers::accounts::SessionBridge for Accounts {
    async fn call(&self, service: &str, op: &str, args: Value) -> Result<Value> {
        self.run(service, op, args, Duration::from_secs(25)).await
    }
}

/// The services, for Settings → Accounts.
pub fn list() -> Vec<Value> {
    SERVICES.iter().map(|s| serde_json::json!({ "id": s.id, "name": s.name })).collect()
}
