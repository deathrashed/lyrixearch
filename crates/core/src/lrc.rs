//! Small helpers for LRC (`[mm:ss.xx]text`) text.

use once_cell::sync::Lazy;
use regex::Regex;

static TIMESTAMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?(?:-\d+)?\]").unwrap());
/// Enhanced LRC's word stamps: `[00:12.30]<00:12.30>When <00:12.80>you`.
static WORD_STAMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"<\d{1,3}:\d{2}(?:[.:]\d{1,3})?>").unwrap());
static META_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*\[(ar|ti|al|by|offset|length|au|re|ve|id|hash|sign|qq|total|language|#):[^\]]*\]\s*$").unwrap());

/// `[id:$00000000]`, `[hash:...]`, `[total:320317]`: header tags that some sources leave in the text.
static TAG_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\[[A-Za-z#]+:[^\]]*\]\s*$").unwrap());

/// Credit labels some sources put on the first lines of a lyric (lyricist, composer, ...), in
/// English, Chinese and Japanese, as they also turn up in LRCLIB entries.
static CREDIT_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^\s*(作词|作詞|作曲|编曲|編曲|制作人|製作人|制作|监制|混音|母带|录音|演唱|歌手|原唱|和声|词|曲|纯音乐，?请欣赏|纯音乐|此歌曲为没有填词的纯音乐)\s*([:：].*)?$").unwrap()
});

/// Drop the credit lines at the top of a lyric. Some tracks have nothing else (instrumentals), and
/// those must not count as lyrics.
pub fn strip_credits(text: &str) -> String {
    let lines = text.lines().skip_while(|l| l.trim().is_empty() || CREDIT_LINE.is_match(l) || TAG_LINE.is_match(l)).collect::<Vec<_>>().join("\n");
    lines.trim().to_string()
}

/// True when there is enough text to be a real lyric: two lines, or about a line's worth of characters.
pub fn is_substantial(text: &str) -> bool {
    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    lines.len() >= 2 || lines.iter().map(|l| l.chars().count()).sum::<usize>() >= 20
}

pub fn looks_synced(s: &str) -> bool {
    TIMESTAMP.is_match(s)
}

/// Remove timestamps and header tags, leaving plain lines.
pub fn strip_timestamps(lrc: &str) -> String {
    lrc.lines()
        .filter(|l| !META_LINE.is_match(l))
        .map(|l| WORD_STAMP.replace_all(&TIMESTAMP.replace_all(l, ""), "").split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// True when the LRC has timing for single words (Enhanced LRC).
pub fn has_word_timing(lrc: &str) -> bool {
    WORD_STAMP.is_match(lrc)
}

/// Enhanced LRC -> ordinary LRC: the line stamps stay, the word stamps go.
pub fn strip_word_stamps(lrc: &str) -> String {
    lrc.lines()
        .map(|l| {
            let cleaned = WORD_STAMP.replace_all(l, "");
            // keep the leading line stamps exactly; tidy the spacing of the text after them
            let text_start =
                TIMESTAMP.find_iter(&cleaned).take_while(|m| m.start() == 0 || cleaned[..m.start()].trim().is_empty()).last().map_or(0, |m| m.end());
            format!("{}{}", &cleaned[..text_start], cleaned[text_start..].split_whitespace().collect::<Vec<_>>().join(" "))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// One `[mm:ss.xx]` stamp, captured for [`timed_lines`]: minutes, seconds, optional fraction.
static STAMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[(\d{1,3}):(\d{2})(?:[.:](\d{1,3}))?\]").unwrap());
static OFFSET: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?i)^\s*\[offset:\s*([+-]?\d+)\s*\]\s*$").unwrap());

/// LRC text as `(milliseconds, line)` pairs sorted by time, for ID3 `SYLT` frames.
///
/// A line with several stamps (`[00:10.00][01:20.00]chorus`) yields one entry per stamp, and an
/// `[offset:+/-ms]` header is applied (positive means the lyric shows earlier, as in the LRC
/// convention). Header lines and lines without a stamp are skipped; empty lyric lines are kept as
/// blank entries so a player can clear the previous line.
pub fn timed_lines(lrc: &str) -> Vec<(u32, String)> {
    let offset: i64 = lrc.lines().find_map(|l| OFFSET.captures(l)).and_then(|c| c[1].parse().ok()).unwrap_or(0);
    let mut out = Vec::new();
    for line in lrc.lines() {
        if META_LINE.is_match(line) {
            continue;
        }
        let text = WORD_STAMP.replace_all(&STAMP.replace_all(line, ""), "").split_whitespace().collect::<Vec<_>>().join(" ");
        for cap in STAMP.captures_iter(line) {
            let (Ok(min), Ok(sec)) = (cap[1].parse::<i64>(), cap[2].parse::<i64>()) else { continue };
            // ".5" is tenths, ".50" hundredths, ".500" milliseconds
            let frac_ms = cap.get(3).map_or(0, |f| {
                let digits = f.as_str();
                let n: i64 = digits.parse().unwrap_or(0);
                match digits.len() {
                    1 => n * 100,
                    2 => n * 10,
                    _ => n,
                }
            });
            let ms = (min * 60_000 + sec * 1000 + frac_ms - offset).clamp(0, i64::from(u32::MAX));
            out.push((ms as u32, text.clone()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_stamps_are_kept_apart_from_line_stamps() {
        let e = "[ti:Song]\n[00:12.30]<00:12.30>When <00:12.80>you <00:13.10>were\n[00:15.00]<00:15.00>Here";
        assert!(has_word_timing(e));
        assert_eq!(strip_word_stamps(e), "[ti:Song]\n[00:12.30]When you were\n[00:15.00]Here");
        assert_eq!(strip_timestamps(e), "When you were\nHere");
        assert_eq!(timed_lines(e), vec![(12300, "When you were".to_string()), (15000, "Here".to_string())]);
        assert!(!has_word_timing("[00:01.00]plain line"));
    }

    #[test]
    fn credits_only_is_not_a_lyric() {
        let raw = "[00:00.00-1] 作词 : Анатолий Чеботарь\n[00:00.00-1] 作曲 : Анатолий Чеботарь\n";
        assert!(looks_synced(raw));
        let plain = strip_credits(&strip_timestamps(raw));
        assert_eq!(plain, "");
        assert!(!is_substantial(&plain));
        let real = strip_credits("作词 : A\n作曲 : B\nfirst line\nsecond line");
        assert_eq!(real, "first line\nsecond line");
        assert!(is_substantial(&real));
    }

    #[test]
    fn kugou_instrumental_placeholder_is_not_a_lyric() {
        let raw = "[id:$00000000]\n[hash:4399c9872c7235b60b58ce88dc487897]\n[sign:]\n[qq:]\n[total:320317]\n纯音乐，请欣赏";
        assert!(!is_substantial(&strip_credits(raw)));
        assert!(!is_substantial(&strip_credits(&strip_timestamps(raw))));
    }

    #[test]
    fn timed_lines_parse_stamps_fractions_and_offset() {
        let lrc = "[ar:Someone]\n[offset:+500]\n[00:01.5]tenths\n[00:02.25]hundredths\n[00:03.125]millis\n[00:10.00][00:04.00]twice\nno stamp\n[00:00.20]";
        let got = timed_lines(lrc);
        assert_eq!(
            got,
            vec![
                (0, String::new()), // 200 ms minus the 500 ms offset clamps to 0
                (1000, "tenths".into()),
                (1750, "hundredths".into()),
                (2625, "millis".into()),
                (3500, "twice".into()),
                (9500, "twice".into()),
            ]
        );
        assert!(timed_lines("just text").is_empty());
    }

    #[test]
    fn strips() {
        let lrc = "[ar:Someone]\n[00:01.00]first line\n[00:05.50]second line";
        assert_eq!(strip_timestamps(lrc), "first line\nsecond line");
        assert!(looks_synced(lrc));
        assert!(!looks_synced("plain"));
    }
}
