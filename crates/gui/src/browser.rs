//! A hidden browser window that loads pages for sources behind bot checks (Cloudflare and the like).
//!
//! Those checks look at the browser itself, not just a cookie, so a plain HTTP client stays blocked
//! even after a check was passed. A real browser engine usually passes the "Verifying…" step by
//! itself, so pages are loaded here, out of sight, and their HTML is handed to the source's parser.
//! Only when a check truly needs a person does the app say so (`check-needed`), and the window is
//! shown when the person chooses to (`show`). Nothing ever pops up or takes focus on its own.

use crate::tray;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WindowEvent};

/// Window label of the hidden browser.
pub const LABEL: &str = "check";

/// The identity the hidden browser presents. It must match the engine underneath: a WebKit view
/// that claims to be Chrome is caught by Cloudflare and then "verifies" forever.
#[cfg(target_os = "macos")]
pub(crate) const BROWSER_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";
#[cfg(target_os = "windows")]
pub(crate) const BROWSER_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36 Edg/131.0.0.0";
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub(crate) const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15";

/// How long a check may try to pass by itself before the person is told.
const QUIET: Duration = Duration::from_secs(12);
/// How long one page may take, a check the person is solving included.
const PAGE_LIMIT: Duration = Duration::from_secs(45);

/// Sent to the UI while a check is waiting for the person, and when it is over.
#[derive(Clone, Serialize)]
pub struct CheckInfo {
    pub host: String,
}

/// What the page reports about itself (see [`PROBE_JS`]).
#[derive(Deserialize, Default)]
struct Probe {
    /// Start time of the document; differs for every new page.
    origin: f64,
    ready: String,
    challenge: bool,
    body: Option<String>,
    #[serde(default)]
    why: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    href: String,
}

/// Reports the page state; `__TEXT__` is replaced by `true` for text (JSON endpoints) or `false` for HTML.
///
/// A challenge is recognised by what the person would see (the challenge title, form or Cloudflare's
/// challenge frame), not by Cloudflare's script variables: those stay on a normal page after a pass.
/// The page counts as loaded once its HTML is parsed ("interactive"); many sites never reach
/// "complete" because ads keep loading. `origin` (the document's start time) tells a new document
/// from the one that was there before the navigation.
const PROBE_JS: &str = r#"(() => {
  const t = document.title || '';
  const byTitle = /just a moment|verify you are human|attention required|checking your browser|security check/i.test(t);
  // Cloudflare, DuckDuckGo's "anomaly" check, and reCAPTCHA / hCaptcha walls (AZLyrics and others)
  const byElement = !!document.querySelector('#challenge-form, #challenge-stage, #challenge-running, #cf-challenge-running, iframe[src*="challenges.cloudflare.com"], .anomaly-modal__modal, .anomaly-modal__mask, form[action*="anomaly"], .g-recaptcha, .h-captcha, iframe[src*="recaptcha"], iframe[src*="hcaptcha"]');
  const challenge = byTitle || byElement;
  const ready = document.readyState;
  const done = ready !== 'loading' && !challenge && !!document.body;
  return { origin: performance.timeOrigin, ready, challenge, why: [byTitle && 'title', byElement && 'element'].filter(Boolean).join(','),
    title: t.slice(0, 80), href: location.href.slice(0, 120),
    body: done ? (__TEXT__ ? document.body.innerText : document.documentElement.outerHTML) : null };
})()"#;

pub struct HiddenBrowser {
    app: AppHandle,
    /// One page at a time: the window has one document.
    turn: tokio::sync::Mutex<()>,
}

impl HiddenBrowser {
    pub fn new(app: AppHandle) -> Self {
        Self { app, turn: tokio::sync::Mutex::new(()) }
    }

    /// The hidden window, created on first use. Closing it only hides it, so it can be reused.
    fn window(&self) -> Result<WebviewWindow> {
        if let Some(w) = self.app.get_webview_window(LABEL) {
            return Ok(w);
        }
        let w = tauri::WebviewWindowBuilder::new(&self.app, LABEL, WebviewUrl::External("about:blank".parse()?))
            .title("Lyrixearch: quick check")
            .inner_size(520.0, 640.0)
            .center()
            .visible(false)
            .focused(false)
            .user_agent(BROWSER_UA)
            .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/128x128.png"))?)?
            .build()?;
        let hide = w.clone();
        w.on_window_event(move |ev| {
            if let WindowEvent::CloseRequested { api, .. } = ev {
                api.prevent_close();
                let _ = hide.hide();
            }
        });
        Ok(w)
    }

    /// Run JS in the page and read its JSON result back.
    async fn probe(w: &WebviewWindow, text: bool) -> Option<Probe> {
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let tx = std::sync::Mutex::new(Some(tx));
        let js = PROBE_JS.replace("__TEXT__", if text { "true" } else { "false" });
        w.eval_with_callback(js, move |json| {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(json);
            }
        })
        .ok()?;
        let json = tokio::time::timeout(Duration::from_secs(5), rx).await.ok()?.ok()?;
        serde_json::from_str(&json).ok()
    }

    /// Show the waiting check: the person chose to pass it.
    pub fn show(app: &AppHandle) -> Result<()> {
        let w = app.get_webview_window(LABEL).context("no check is waiting")?;
        w.show()?;
        w.set_focus()?;
        Ok(())
    }

    /// Put the window away again (the person dismissed the check).
    pub fn hide(app: &AppHandle) {
        if let Some(w) = app.get_webview_window(LABEL) {
            let _ = w.hide();
        }
    }
}

#[async_trait::async_trait]
impl lyrixearch_core::providers::web::BrowserFetch for HiddenBrowser {
    async fn fetch(&self, url: &str, text: bool) -> Result<String> {
        let _turn = self.turn.lock().await;
        let host = url.split('/').nth(2).unwrap_or(url).to_string();
        let w = self.window()?;
        // the document now in the window, so its leftovers are not mistaken for the new page
        let before = Self::probe(&w, text).await.map_or(-1.0, |p| p.origin);
        w.navigate(url.parse().with_context(|| format!("bad url {url}"))?)?;
        let started = Instant::now();
        let mut asked = false;
        loop {
            tokio::time::sleep(Duration::from_millis(400)).await;
            let p = Self::probe(&w, text).await.unwrap_or_default();
            if std::env::var_os("LYRIXEARCH_BROWSER_DEBUG").is_some() {
                eprintln!(
                    "browser {:>5.1}s ready={} fresh={} challenge={} why={} title={:?} href={}",
                    started.elapsed().as_secs_f32(),
                    p.ready,
                    p.origin != before,
                    p.challenge,
                    p.why,
                    p.title,
                    p.href
                );
            }
            let fresh = p.origin > 0.0 && p.origin != before;
            if let Some(body) = p.body.filter(|_| fresh) {
                eprintln!("lyrixearch: {host} page loaded in the hidden browser in {:.1}s", started.elapsed().as_secs_f32());
                if asked {
                    // the person passed it: put the window away and say so
                    let _ = w.hide();
                    let _ = self.app.emit("check-cleared", CheckInfo { host: host.clone() });
                    tray::set(&self.app, tray::TrayState::Fetching, "fetching lyrics");
                }
                return Ok(body);
            }
            if p.challenge && fresh && !asked && started.elapsed() >= QUIET {
                asked = true;
                eprintln!("lyrixearch: {host} check did not pass by itself, asking the person");
                let _ = self.app.emit("check-needed", CheckInfo { host: host.clone() });
                tray::set(&self.app, tray::TrayState::Error, format!("{host} needs a quick human check, open Lyrixearch"));
            }
            if started.elapsed() >= PAGE_LIMIT {
                if p.challenge {
                    return Err(lyrixearch_core::providers::Challenge { url: url.to_string(), reason: format!("{host} needs a quick human check") }.into());
                }
                return Err(anyhow!("{host}: page did not finish loading (state {})", p.ready));
            }
        }
    }
}
