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
    fn feat() {
        assert_eq!(strip_featuring("Song (feat. Someone)"), "Song");
    }
}
