//! Title/artist clean-up applied before searching and before comparing candidates.

use once_cell::sync::Lazy;
use regex::Regex;

/// Trailing "(Remastered 2011)", "[Live at ...]", "(Official Video)" style noise.
static BRACKET_NOISE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\s*[\(\[][^\)\]]*\b(official|lyrics?|video|audio|explicit|clean|remaster(?:ed)?|re-?issue(?:d)?|re-?recorded|remix(?:ed)?|live|deluxe|edition|bonus|mono|stereo|hd|hq|version)\b[^\)\]]*[\)\]]\s*$",
    )
    .unwrap()
});

/// "Song - 2011 Remaster" / "Song - Live" style suffixes.
static DASH_NOISE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)\s+-\s+(?:\d{4}\s+)?(?:remaster(?:ed)?|re-?issue(?:d)?|live|mono|stereo|single version|radio edit)\b.*$").unwrap());

static FEAT: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)\s*[\(\[]?\b(?:feat\.?|ft\.?|featuring)\s+[^\)\]]*[\)\]]?").unwrap());

pub fn clean_title(title: &str) -> String {
    let mut s = title.trim().to_string();
    loop {
        let next = BRACKET_NOISE.replace(&s, "").into_owned();
        let next = DASH_NOISE.replace(&next, "").into_owned();
        if next == s {
            break;
        }
        s = next;
    }
    s.trim().to_string()
}

/// Any trailing bracketed parts at all: "Song (Part 2) [Demo]" -> "Song". The original uses this
/// as a last retry when the cleaned title finds nothing.
static TRAILING_BRACKETS: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?:\s*[\(\[][^\)\]]*[\)\]])+\s*$").unwrap());

pub fn strip_trailing_brackets(title: &str) -> String {
    TRAILING_BRACKETS.replace(title.trim(), "").trim().to_string()
}

/// Placeholder titles and artists that are not worth searching for ("Track 01", "Unknown Artist").
static PLACEHOLDER_TITLE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*(?:track\s*\d*|audio\s*track\s*\d*|untitled(?:\s*\d+)?|unknown(?:\s*title)?|\d+|[-_.\s]*)\s*$").unwrap());
static PLACEHOLDER_ARTIST: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*(?:unknown(?:\s*artist)?|artist|<unknown>|[-_.\s]*)\s*$").unwrap());

/// Why a track is not worth sending to online sources (the original's suitability check), or
/// `None` when it is. Local files next to the track are still looked at.
pub fn unsuitable(artist: &str, title: &str) -> Option<&'static str> {
    if PLACEHOLDER_TITLE.is_match(title) {
        Some("the title is empty or a placeholder")
    } else if PLACEHOLDER_ARTIST.is_match(artist) {
        Some("the artist is empty or a placeholder")
    } else {
        None
    }
}

/// Strip any "feat. X" tail from an artist or title string.
pub fn strip_featuring(s: &str) -> String {
    FEAT.replace_all(s, "").trim().to_string()
}

/// Split `Artist - Title` file stems; used when tags are empty.
pub fn guess_from_filename(stem: &str) -> Option<(String, String)> {
    // drop a leading track number like "01 - " / "01. "
    static LEADING_NUM: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\d{1,3}\s*[-._)]\s*").unwrap());
    let stem = LEADING_NUM.replace(stem, "");
    let (a, t) = stem.split_once(" - ")?;
    let (a, t) = (a.trim(), t.trim());
    if a.is_empty() || t.is_empty() {
        None
    } else {
        Some((a.to_string(), t.to_string()))
    }
}

/// Placeholder artists for which the album artist should be tried too.
pub fn is_various_artists(artist: &str) -> bool {
    matches!(artist.trim().to_ascii_lowercase().as_str(), "various artists" | "various" | "va")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_noise() {
        assert_eq!(clean_title("Song (Remastered 2011)"), "Song");
        assert_eq!(clean_title("Song [Live at Wembley] (Official Video)"), "Song");
        assert_eq!(clean_title("Song - 2009 Remaster"), "Song");
        assert_eq!(clean_title("Song (Part 2)"), "Song (Part 2)");
    }

    #[test]
    fn filename_guess() {
        assert_eq!(guess_from_filename("01 - Artist - Title"), Some(("Artist".into(), "Title".into())));
        assert_eq!(guess_from_filename("justatitle"), None);
    }

    #[test]
    fn trailing_brackets() {
        assert_eq!(strip_trailing_brackets("Song (Part 2) [Demo]"), "Song");
        assert_eq!(strip_trailing_brackets("Song (Part 2)"), "Song");
        assert_eq!(strip_trailing_brackets("(Intro) Song"), "(Intro) Song");
        assert_eq!(strip_trailing_brackets("Song"), "Song");
    }

    #[test]
    fn suitability() {
        assert!(unsuitable("Queen", "Bohemian Rhapsody").is_none());
        for t in ["", "  ", "Track 01", "track", "Audio Track 3", "Untitled", "unknown", "07"] {
            assert!(unsuitable("Queen", t).is_some(), "title {t:?}");
        }
        for a in ["", "Unknown Artist", "unknown", "<unknown>"] {
            assert!(unsuitable(a, "Song").is_some(), "artist {a:?}");
        }
        assert!(unsuitable("Artist 2", "Track Two Of The Night").is_none());
    }

    #[test]
    fn feat() {
        assert_eq!(strip_featuring("Song (feat. Someone)"), "Song");
    }
}
