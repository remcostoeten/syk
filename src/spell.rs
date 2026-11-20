use std::cmp::Reverse;
use std::collections::HashMap;
use std::io;

use grep_matcher::Matcher;
use grep_regex::RegexMatcher;
use grep_searcher::{Searcher, Sink, SinkMatch};

const SHORTEST: usize = 4;
const LONGEST: usize = 40;
const WILDCARD: &str = r"[\w-]";

#[derive(Default)]
pub struct Tally {
    pub total: u64,
    pub whole: u64,
}

pub type Seen = HashMap<String, Tally>;

pub fn correctable(source: &str) -> bool {
    let length = source.chars().count();
    (SHORTEST..=LONGEST).contains(&length)
        && source
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
        && source.chars().any(char::is_alphanumeric)
}

pub fn variants(source: &str) -> String {
    let letters: Vec<char> = source.chars().collect();
    let text = |range: std::ops::Range<usize>| -> String { letters[range].iter().collect() };
    let count = letters.len();
    let mut found: Vec<String> = Vec::new();
    for at in 1..count {
        found.push(format!("{}{WILDCARD}{}", text(0..at), text(at..count)));
    }
    for at in 0..count - 1 {
        if letters[at] != letters[at + 1] {
            found.push(format!(
                "{}{}{}{}",
                text(0..at),
                letters[at + 1],
                letters[at],
                text(at + 2..count)
            ));
        }
    }
    for at in 0..count {
        found.push(format!("{}{WILDCARD}{}", text(0..at), text(at + 1..count)));
    }
    if count > SHORTEST {
        for at in 0..count {
            let shorter = format!("{}{}", text(0..at), text(at + 1..count));
            if !found.contains(&shorter) {
                found.push(shorter);
            }
        }
    }
    found.join("|")
}

pub fn note(seen: &mut Seen, haystack: &[u8], start: usize, end: usize, fold: bool) {
    let Ok(text) = std::str::from_utf8(&haystack[start..end]) else {
        return;
    };
    let key = if fold {
        text.to_lowercase()
    } else {
        text.to_string()
    };
    let tally = seen.entry(key).or_default();
    tally.total += 1;
    if boundary(haystack, start) && boundary(haystack, end) {
        tally.whole += 1;
    }
}

pub fn merge(into: &mut Seen, from: &mut Seen) {
    for (text, tally) in from.drain() {
        let total = into.entry(text).or_default();
        total.total += tally.total;
        total.whole += tally.whole;
    }
}

pub fn best(seen: Seen, source: &str) -> Option<String> {
    let length = source.chars().count();
    let folded = source.to_lowercase();
    seen.into_iter()
        .filter(|(text, _)| text.to_lowercase() != folded)
        .filter(|(text, tally)| tally.whole > 0 || text.chars().count() >= length)
        .max_by_key(|(text, tally)| {
            (
                tally.whole > 0,
                text.chars().count() >= length,
                tally.total,
                Reverse(text.clone()),
            )
        })
        .map(|(text, _)| text)
}

fn boundary(haystack: &[u8], at: usize) -> bool {
    if at == 0 || at >= haystack.len() {
        return true;
    }
    let (before, after) = (haystack[at - 1], haystack[at]);
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte >= 0x80;
    !word(before) || !word(after) || (before.is_ascii_lowercase() && after.is_ascii_uppercase())
}

pub struct Gather<'a> {
    pub matcher: &'a RegexMatcher,
    pub seen: &'a mut Seen,
    pub fold: bool,
    pub binary: bool,
}

impl Sink for Gather<'_> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        let line = found.bytes();
        let (seen, fold) = (&mut *self.seen, self.fold);
        let _ = self.matcher.find_iter(line, |hit| {
            if !hit.is_empty() {
                note(seen, line, hit.start(), hit.end(), fold);
            }
            true
        });
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, io::Error> {
        self.binary = true;
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen(rows: &[(&str, u64, u64)]) -> Seen {
        rows.iter()
            .map(|(text, total, whole)| {
                (
                    text.to_string(),
                    Tally {
                        total: *total,
                        whole: *whole,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn only_plain_words_are_corrected() {
        assert!(correctable("confg"));
        assert!(correctable("use client"));
        assert!(correctable("my-file_name"));
        assert!(!correctable("abc"));
        assert!(!correctable("foo.*bar"));
        assert!(!correctable("fn main("));
        assert!(!correctable("----"));
    }

    #[test]
    fn variants_cover_one_edit() {
        let regex = variants("confg");
        for expected in [r"conf[\w-]g", "cnofg", r"co[\w-]fg", "cofg"] {
            assert!(
                regex.split('|').any(|part| part == expected),
                "{expected} missing"
            );
        }
        assert!(!variants("tets").split('|').any(|part| part == "tes"));
    }

    #[test]
    fn whole_words_beat_fragments() {
        let found = best(
            seen(&[("confi", 40, 0), ("config", 12, 9), ("conf", 30, 2)]),
            "confg",
        );
        assert_eq!(found.as_deref(), Some("config"));
    }

    #[test]
    fn shorter_fragments_are_never_suggested() {
        assert_eq!(best(seen(&[("conf", 30, 0)]), "confg"), None);
        assert_eq!(
            best(seen(&[("conf", 30, 1)]), "confg").as_deref(),
            Some("conf")
        );
        assert_eq!(best(seen(&[("CONFG", 3, 3)]), "confg"), None);
    }

    #[test]
    fn boundaries_include_case_changes_and_underscores() {
        let mut found = Seen::new();
        note(&mut found, b"getConfig()", 3, 9, true);
        note(&mut found, b"my_config_file", 3, 9, true);
        note(&mut found, b"reconfigure", 2, 8, true);
        let tally = &found["config"];
        assert_eq!((tally.total, tally.whole), (3, 2));
    }
}
