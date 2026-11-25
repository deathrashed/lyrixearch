//! Web-search fallback: ask DuckDuckGo for `"artist title lyrics"` and hand the result URLs to
//! whichever scraping provider recognises them (the original's "DuckDuckGo (search)" source).

use super::web::get_html;
use super::Provider;
use crate::config::Config;
use crate::model::{Lyrics, Query};
use anyhow::Result;
use async_trait::async_trait;
use once_cell::sync::Lazy;
use regex::Regex;
use std::sync::Arc;

pub struct DuckDuckGo {
    pub scrapers: Vec<Arc<dyn Provider>>,
}

static RESULT_LINK: Lazy<Regex> = Lazy::new(|| Regex::new(r#"class="result__a"[^>]*href="([^"]+)""#).unwrap());

/// DDG wraps targets as `//duckduckgo.com/l/?uddg=<urlencoded>&rut=...`.
fn unwrap_redirect(href: &str) -> String {
    let href = html_escape::decode_html_entities(href).into_owned();
    if let Some(i) = href.find("uddg=") {
        let rest = &href[i + 5..];
        let enc = rest.split('&').next().unwrap_or(rest);
        return urlencoding::decode(enc).map(|c| c.into_owned()).unwrap_or_else(|_| enc.to_string());
    }
    href
}

#[async_trait]
impl Provider for DuckDuckGo {
    fn id(&self) -> &'static str {
        "ddg_search"
    }

    async fn fetch(&self, http: &reqwest::Client, q: &Query, cfg: &Config) -> Result<Option<Lyrics>> {
        let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding::encode(&format!("{} {} lyrics", q.artist, q.title)));
        // a captcha page comes back as a Challenge: the site is then searched through the app's
        // browser, where the person can answer it once (Lyrics Finder shows a captcha dialog the same way)
        let html = get_html(http, &url, &[("Referer", "https://html.duckduckgo.com/")]).await?;
        let links: Vec<String> = RESULT_LINK.captures_iter(&html).map(|c| unwrap_redirect(&c[1])).take(8).collect();
        for link in links {
            for p in &self.scrapers {
                if p.supports_url(&link) {
                    if let Ok(Some(l)) = p.fetch_url(http, &link, q, cfg).await {
                        return Ok(Some(l));
                    }
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unwraps() {
        let h = "//duckduckgo.com/l/?uddg=https%3A%2F%2Fgenius.com%2FQueen-song-lyrics&amp;rut=abc";
        assert_eq!(unwrap_redirect(h), "https://genius.com/Queen-song-lyrics");
    }
}
