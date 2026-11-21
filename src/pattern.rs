use grep_matcher::{LineTerminator, Match, Matcher, NoCaptures, NoError};
use grep_regex::{RegexMatcher, RegexMatcherBuilder};

use crate::query::{Case, Matching};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    Regex,
    Text,
    Glob,
}

pub struct Term {
    pub source: String,
    pub reading: Reading,
    lines: RegexMatcher,
    names: Option<RegexMatcher>,
}

impl Term {
    fn compile(source: &str, matching: &Matching) -> Result<Term, String> {
        let plain = |reading, names| -> Result<Term, String> {
            Ok(Term {
                source: source.to_string(),
                reading,
                lines: build(source, matching, true).map_err(|error| error.to_string())?,
                names,
            })
        };
        if matching.literal {
            return plain(Reading::Text, None);
        }
        match build(source, matching, false) {
            Ok(lines) => Ok(Term {
                source: source.to_string(),
                reading: Reading::Regex,
                lines,
                names: None,
            }),
            Err(_) if source.contains(['*', '?']) => {
                let names = build(&glob(source), matching, false).ok();
                let reading = if names.is_some() {
                    Reading::Glob
                } else {
                    Reading::Text
                };
                plain(reading, names)
            }
            Err(_) => plain(Reading::Text, None),
        }
    }

    fn find(&self, name: &[u8]) -> Option<(usize, usize)> {
        let matcher = self.names.as_ref().unwrap_or(&self.lines);
        match matcher.find(name) {
            Ok(Some(found)) => Some((found.start(), found.end())),
            _ => None,
        }
    }

    fn either(&self) -> String {
        match self.reading {
            Reading::Regex => format!("(?:{})", self.source),
            Reading::Text | Reading::Glob => escape(&self.source),
        }
    }
}

pub struct Pattern {
    terms: Vec<Term>,
    any: Option<RegexMatcher>,
    rarest: usize,
}

impl Pattern {
    pub fn compile(sources: &[String], matching: &Matching) -> Result<Pattern, String> {
        if let [source] = sources {
            return Pattern::one(source, matching);
        }
        let fixed = Matching {
            case: match matching.case {
                Case::Smart if sources.iter().any(|source| has_uppercase(source)) => {
                    Case::Sensitive
                }
                Case::Smart => Case::Insensitive,
                case => case,
            },
            literal: matching.literal,
            word: matching.word,
        };
        let terms = sources
            .iter()
            .map(|source| Term::compile(source, &fixed))
            .collect::<Result<Vec<Term>, String>>()?;
        let either: Vec<String> = terms.iter().map(Term::either).collect();
        let any = build(&either.join("|"), &fixed, false).map_err(|error| error.to_string())?;
        let rarest = (0..terms.len())
            .max_by_key(|index| terms[*index].source.len())
            .unwrap_or(0);
        Ok(Pattern {
            terms,
            any: Some(any),
            rarest,
        })
    }

    pub fn one(source: &str, matching: &Matching) -> Result<Pattern, String> {
        Ok(Pattern {
            terms: vec![Term::compile(source, matching)?],
            any: None,
            rarest: 0,
        })
    }

    pub fn terms(&self) -> &[Term] {
        &self.terms
    }

    pub fn single(&self) -> bool {
        self.any.is_none()
    }

    pub fn find(&self, name: &[u8]) -> Option<(usize, usize)> {
        let (first, rest) = self.terms.split_first()?;
        let found = first.find(name)?;
        rest.iter()
            .all(|term| term.find(name).is_some())
            .then_some(found)
    }

    pub fn lines(&self) -> &RegexMatcher {
        self.any.as_ref().unwrap_or(&self.terms[0].lines)
    }

    pub fn any_case(&self, matching: &Matching) -> bool {
        match matching.case {
            Case::Insensitive => true,
            Case::Sensitive => false,
            Case::Smart => !self.terms.iter().any(|term| has_uppercase(&term.source)),
        }
    }

    fn whole(&self, line: &[u8]) -> bool {
        self.terms
            .iter()
            .enumerate()
            .all(|(index, term)| index == self.rarest || term.lines.is_match(line).unwrap_or(false))
    }
}

impl Matcher for Pattern {
    type Captures = NoCaptures;
    type Error = NoError;

    fn find_at(&self, haystack: &[u8], mut at: usize) -> Result<Option<Match>, NoError> {
        let scan = &self.terms[self.rarest].lines;
        while at <= haystack.len() {
            let from = memchr::memrchr(b'\n', &haystack[..at]).map_or(0, |index| index + 1);
            let Ok(Some(found)) = scan.find_at(haystack, from) else {
                break;
            };
            let start =
                memchr::memrchr(b'\n', &haystack[..found.start()]).map_or(0, |index| index + 1);
            let end = memchr::memchr(b'\n', &haystack[found.end()..])
                .map_or(haystack.len(), |index| found.end() + index);
            if self.whole(&haystack[start..end])
                && let Ok(Some(first)) = self.lines().find_at(&haystack[..end], at.max(start))
            {
                return Ok(Some(first));
            }
            at = end + 1;
        }
        Ok(None)
    }

    fn new_captures(&self) -> Result<NoCaptures, NoError> {
        Ok(NoCaptures::new())
    }

    fn line_terminator(&self) -> Option<LineTerminator> {
        Some(LineTerminator::byte(b'\n'))
    }
}

fn build(
    source: &str,
    matching: &Matching,
    literal: bool,
) -> Result<RegexMatcher, grep_regex::Error> {
    let mut builder = RegexMatcherBuilder::new();
    match matching.case {
        Case::Smart => builder.case_smart(true),
        Case::Insensitive => builder.case_insensitive(true),
        Case::Sensitive => &mut builder,
    };
    builder
        .word(matching.word)
        .fixed_strings(literal)
        .line_terminator(Some(b'\n'))
        .build(source)
}

fn special(character: char) -> bool {
    matches!(
        character,
        '\\' | '.'
            | '+'
            | '*'
            | '?'
            | '('
            | ')'
            | '|'
            | '['
            | ']'
            | '{'
            | '}'
            | '^'
            | '$'
            | '#'
            | '&'
            | '-'
            | '~'
    )
}

fn escape(source: &str) -> String {
    let mut regex = String::with_capacity(source.len() + 8);
    for character in source.chars() {
        if special(character) {
            regex.push('\\');
        }
        regex.push(character);
    }
    regex
}

fn glob(source: &str) -> String {
    let mut regex = String::with_capacity(source.len() + 8);
    regex.push('^');
    for character in source.chars() {
        match character {
            '*' => regex.push_str(".*"),
            '?' => regex.push('.'),
            other if special(other) => {
                regex.push('\\');
                regex.push(other);
            }
            other => regex.push(other),
        }
    }
    regex.push('$');
    regex
}

fn has_uppercase(source: &str) -> bool {
    let mut escaped = false;
    for character in source.chars() {
        if escaped {
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character.is_uppercase() {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use grep_matcher::Matcher;

    use super::*;

    fn both(sources: &[&str]) -> Pattern {
        let sources: Vec<String> = sources.iter().map(|source| source.to_string()).collect();
        Pattern::compile(&sources, &Matching::default()).expect("a pattern")
    }

    #[test]
    fn every_term_has_to_be_in_the_name() {
        let pattern = both(&["route", "api"]);
        assert_eq!(pattern.find(b"api-route.ts"), Some((4, 9)));
        assert_eq!(pattern.find(b"route.ts"), None);
        assert!(!pattern.single());
    }

    #[test]
    fn every_term_has_to_be_on_the_same_line() {
        let pattern = both(&["needle", "a(b"]);
        let text = b"needle alone\na(b alone\nA(B then NEEDLE\n";
        let found = pattern.find_at(text, 0).expect("no error").expect("a hit");
        assert_eq!(&text[found.start()..found.end()], b"A(B");
        assert!(
            pattern
                .find_at(b"needle\na(b\n", 0)
                .expect("no error")
                .is_none()
        );
    }

    #[test]
    fn every_word_on_a_matching_line_is_found_in_turn() {
        let pattern = both(&["pub", "unsafe"]);
        let text = b"pub fn alone\npub x unsafe y pub\nunsafe alone\n";
        let mut found = Vec::new();
        let mut at = 0;
        while let Some(hit) = pattern.find_at(text, at).expect("no error") {
            found.push((hit.start(), &text[hit.start()..hit.end()]));
            at = hit.end();
        }
        assert_eq!(
            found,
            [(13, &b"pub"[..]), (19, &b"unsafe"[..]), (28, &b"pub"[..])]
        );
        assert_eq!(pattern.rarest, 1);
    }

    #[test]
    fn one_upper_case_letter_makes_every_term_exact() {
        let pattern = both(&["Needle", "route"]);
        assert!(pattern.find(b"Needle-route").is_some());
        assert!(pattern.find(b"Needle-Route").is_none());
    }
}
