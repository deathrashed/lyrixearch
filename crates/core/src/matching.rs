//! Candidate scoring. This is our own design, not a port: normalise both sides, compare
//! with edit-distance similarity, and use duration as a tiebreaker when both sides know it.

use crate::clean::{clean_title, strip_featuring};
use crate::model::Query;

/// Lowercase, drop "feat." tails and punctuation, collapse whitespace.
pub fn normalize(s: &str) -> String {
    let s = strip_featuring(s).to_lowercase().replace('&', " and ");
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            out.push(ch);
            last_space = false;
        } else if !last_space {
            out.push(' ');
            last_space = true;
        }
    }
    out.trim().to_string()
}

fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    if a == b {
        return 1.0;
    }
    let lev = strsim::normalized_levenshtein(&a, &b) as f32;
    // "the beatles" vs "beatles", "song" vs "song pt 2": containment counts for something.
    let contain = if a.contains(&b) || b.contains(&a) {
        let (short, long) = if a.len() < b.len() { (&a, &b) } else { (&b, &a) };
        0.75 + 0.2 * (short.len() as f32 / long.len() as f32)
    } else {
        0.0
    };
    lev.max(contain)
}

#[derive(Clone, Copy, Debug)]
pub struct MatchScore {
    pub title: f32,
    pub artist: f32,
    pub overall: f32,
}

/// Markers that make a recording a different version of the song. A candidate carrying one the
/// query does not is ranked lower, so the studio cut beats "(Live)" / "(Remix)" when both exist.
const VERSION_MARKERS: &[&str] = &["live", "remix", "acoustic", "instrumental", "karaoke", "demo", "cover", "version", "edit"];

fn version_penalty(query_title: &str, cand_title: &str) -> f32 {
    let (q, c) = (query_title.to_lowercase(), cand_title.to_lowercase());
    let has = |s: &str, m: &str| s.split(|ch: char| !ch.is_alphanumeric()).any(|w| w == m);
    let extra = VERSION_MARKERS.iter().filter(|m| has(&c, m) && !has(&q, m)).count();
    0.12 * extra.min(2) as f32
}

pub fn score(q: &Query, cand_artist: &str, cand_title: &str, cand_duration_secs: Option<f64>) -> MatchScore {
    let title = similarity(&clean_title(&q.title), &clean_title(cand_title));
    let artist = similarity(&q.artist, cand_artist);
    let mut overall = 0.6 * title + 0.4 * artist - version_penalty(&q.title, cand_title);
    if let (Some(a), Some(b)) = (q.duration_secs, cand_duration_secs) {
        let diff = (a as f64 - b).abs();
        if diff <= 3.0 {
            overall += 0.05;
        } else if diff > 10.0 {
            overall -= 0.15;
        }
    }
    MatchScore { title, artist, overall: overall.clamp(0.0, 1.0) }
}

impl MatchScore {
    /// Both fields must be plausible, not just the blend: a perfect title by the wrong artist fails.
    pub fn accepted(&self, min_overall: f32) -> bool {
        self.title >= 0.85 && self.artist >= 0.6 && self.overall >= min_overall
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(artist: &str, title: &str) -> Query {
        Query { artist: artist.into(), title: title.into(), duration_secs: Some(200), ..Default::default() }
    }

    #[test]
    fn exact_and_noisy_match() {
        let s = score(&q("The Band", "Song (Remastered)"), "The Band", "Song", Some(201.0));
        assert!(s.accepted(0.8), "{s:?}");
    }

    #[test]
    fn wrong_artist_rejected() {
        let s = score(&q("The Band", "Song"), "Someone Else", "Song", None);
        assert!(!s.accepted(0.8), "{s:?}");
    }

    #[test]
    fn different_song_rejected() {
        let s = score(&q("The Band", "Song"), "The Band", "Completely Different", None);
        assert!(!s.accepted(0.8), "{s:?}");
    }

    #[test]
    fn studio_beats_live() {
        let studio = score(&q("The Band", "Song"), "The Band", "Song", None);
        let live = score(&q("The Band", "Song"), "The Band", "Song (Live)", None);
        assert!(studio.overall > live.overall);
        assert!(live.accepted(0.8), "live still acceptable when it is all there is: {live:?}");
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize("Rock & Roll (feat. X)"), "rock and roll");
    }
}
