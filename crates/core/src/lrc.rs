//! Small helpers for LRC (`[mm:ss.xx]text`) text.

use once_cell::sync::Lazy;
use regex::Regex;

static TIMESTAMP: Lazy<Regex> = Lazy::new(|| Regex::new(r"\[\d{1,3}:\d{2}(?:[.:]\d{1,3})?(?:-\d+)?\]").unwrap());
static META_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?i)^\s*\[(ar|ti|al|by|offset|length|au|re|ve|id|hash|sign|qq|total|language|#):[^\]]*\]\s*$").unwrap());

/// `[id:$00000000]`, `[hash:...]`, `[total:320317]`: header tags that some sources leave in the text.
static TAG_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*\[[A-Za-z#]+:[^\]]*\]\s*$").unwrap());

/// Credit labels that NetEase, QQ and Kugou put on the first lines of a lyric (lyricist, composer, ...).
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
    lrc.lines().filter(|l| !META_LINE.is_match(l)).map(|l| TIMESTAMP.replace_all(l, "").trim().to_string()).collect::<Vec<_>>().join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn strips() {
        let lrc = "[ar:Someone]\n[00:01.00]first line\n[00:05.50]second line";
        assert_eq!(strip_timestamps(lrc), "first line\nsecond line");
        assert!(looks_synced(lrc));
        assert!(!looks_synced("plain"));
    }
}
