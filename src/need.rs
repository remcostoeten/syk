use std::collections::BTreeSet;

use regex_syntax::ParserBuilder;
use regex_syntax::hir::{Class, Hir, HirKind};

const WIDEST: usize = 16;
const SPREAD: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Need {
    Any,
    Gram(u32),
    All(Vec<Need>),
    One(Vec<Need>),
}

type Texts = BTreeSet<Vec<u8>>;

struct Part {
    exact: Option<Texts>,
    need: Need,
}

impl Part {
    fn exactly(texts: Texts) -> Part {
        Part {
            exact: Some(texts),
            need: Need::Any,
        }
    }

    fn vague(need: Need) -> Part {
        Part { exact: None, need }
    }

    fn settle(self) -> Need {
        match self.exact {
            Some(texts) => strings(&texts),
            None => self.need,
        }
    }
}

pub fn gram(bytes: &[u8]) -> Option<u32> {
    let [first, second, third] = *bytes else {
        return None;
    };
    let lowered = [first, second, third].map(|byte| byte.to_ascii_lowercase());
    (!lowered.contains(&b'\n'))
        .then(|| u32::from(lowered[0]) << 16 | u32::from(lowered[1]) << 8 | u32::from(lowered[2]))
}

pub fn all(needs: Vec<Need>) -> Need {
    let mut kept: Vec<Need> = Vec::new();
    for need in needs {
        match need {
            Need::Any => {}
            Need::All(inner) => kept.extend(inner),
            other => kept.push(other),
        }
    }
    kept.sort_by_key(|need| match need {
        Need::Gram(gram) => (0, *gram),
        _ => (1, 0),
    });
    kept.dedup();
    match kept.len() {
        0 => Need::Any,
        1 => kept.pop().unwrap_or(Need::Any),
        _ => Need::All(kept),
    }
}

fn one(mut needs: Vec<Need>) -> Need {
    needs.dedup();
    if needs.is_empty() || needs.contains(&Need::Any) {
        return Need::Any;
    }
    match needs.len() {
        1 => needs.pop().unwrap_or(Need::Any),
        _ => Need::One(needs),
    }
}

fn strings(texts: &Texts) -> Need {
    one(texts
        .iter()
        .map(|text| all(text.windows(3).filter_map(gram).map(Need::Gram).collect()))
        .collect())
}

fn lone() -> Texts {
    BTreeSet::from([Vec::new()])
}

fn members(class: &Class) -> Option<Texts> {
    let mut texts = BTreeSet::new();
    match class {
        Class::Unicode(class) => {
            let count: usize = class.ranges().iter().map(|range| range.len()).sum();
            if count > WIDEST {
                return None;
            }
            for character in class
                .ranges()
                .iter()
                .flat_map(|range| range.start()..=range.end())
            {
                let mut buffer = [0u8; 4];
                texts.insert(
                    character
                        .encode_utf8(&mut buffer)
                        .as_bytes()
                        .to_ascii_lowercase(),
                );
            }
        }
        Class::Bytes(class) => {
            let count: usize = class.ranges().iter().map(|range| range.len()).sum();
            if count > WIDEST {
                return None;
            }
            for byte in class
                .ranges()
                .iter()
                .flat_map(|range| range.start()..=range.end())
            {
                texts.insert(vec![byte.to_ascii_lowercase()]);
            }
        }
    }
    Some(texts)
}

fn cross(left: &Texts, right: &Texts) -> Texts {
    left.iter()
        .flat_map(|start| {
            right
                .iter()
                .map(move |end| [start.as_slice(), end].concat())
        })
        .collect()
}

fn chain(parts: &[Hir]) -> Part {
    let mut run = lone();
    let mut needs = Vec::new();
    let mut whole = true;
    for hir in parts {
        let next = part(hir);
        match next.exact {
            Some(exact) if run.len() * exact.len() <= SPREAD => run = cross(&run, &exact),
            Some(exact) => {
                needs.push(strings(&run));
                run = exact;
                whole = false;
            }
            None => {
                needs.push(strings(&run));
                needs.push(next.need);
                run = lone();
                whole = false;
            }
        }
    }
    if whole {
        return Part::exactly(run);
    }
    needs.push(strings(&run));
    Part::vague(all(needs))
}

fn either(parts: &[Hir]) -> Part {
    let parts: Vec<Part> = parts.iter().map(part).collect();
    let mut union = BTreeSet::new();
    let exact = parts.iter().all(|part| match &part.exact {
        Some(exact) => {
            union.extend(exact.iter().cloned());
            union.len() <= SPREAD
        }
        None => false,
    });
    if exact {
        return Part::exactly(union);
    }
    Part::vague(one(parts.into_iter().map(Part::settle).collect()))
}

fn part(hir: &Hir) -> Part {
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => Part::exactly(lone()),
        HirKind::Literal(literal) => {
            Part::exactly(BTreeSet::from([literal.0.to_ascii_lowercase()]))
        }
        HirKind::Class(class) => match members(class) {
            Some(texts) => Part::exactly(texts),
            None => Part::vague(Need::Any),
        },
        HirKind::Capture(capture) => part(&capture.sub),
        HirKind::Repetition(repetition) => match (repetition.min, repetition.max) {
            (1, Some(1)) => part(&repetition.sub),
            (0, _) => Part::vague(Need::Any),
            _ => Part::vague(part(&repetition.sub).settle()),
        },
        HirKind::Concat(parts) => chain(parts),
        HirKind::Alternation(parts) => either(parts),
    }
}

pub fn of(source: &str, literal: bool) -> Need {
    let escaped;
    let source = match literal {
        true => {
            escaped = regex_syntax::escape(source);
            escaped.as_str()
        }
        false => source,
    };
    // Any case is read, so the need holds for every way the pattern is matched
    let parsed = ParserBuilder::new()
        .case_insensitive(true)
        .utf8(false)
        .build()
        .parse(source);
    match parsed {
        Ok(hir) => part(&hir).settle(),
        Err(_) => Need::Any,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grams(text: &str) -> Need {
        all(text
            .as_bytes()
            .windows(3)
            .filter_map(gram)
            .map(Need::Gram)
            .collect())
    }

    fn holds(need: &Need, text: &[u8]) -> bool {
        let lowered = text.to_ascii_lowercase();
        match need {
            Need::Any => true,
            Need::Gram(wanted) => lowered.windows(3).any(|found| gram(found) == Some(*wanted)),
            Need::All(needs) => needs.iter().all(|need| holds(need, text)),
            Need::One(needs) => needs.iter().any(|need| holds(need, text)),
        }
    }

    #[test]
    fn plain_text_needs_each_of_its_pieces() {
        assert_eq!(of("needle", false), grams("needle"));
        assert_eq!(of("NeeDle", false), grams("needle"));
        assert_eq!(of("a(b", true), grams("a(b"));
        assert_eq!(of("ab", false), Need::Any);
        assert_eq!(of("", false), Need::Any);
    }

    #[test]
    fn letters_with_more_than_two_cases_keep_every_spelling() {
        let need = of("skip", false);
        assert!(matches!(&need, Need::One(spellings) if spellings.len() == 4));
        for text in ["skip", "SKIP", "ſkip", "S\u{212A}ip"] {
            assert!(holds(&need, text.as_bytes()), "{text} is ruled out");
        }
        assert!(!holds(&need, b"skid"));
    }

    #[test]
    fn a_regex_needs_the_text_around_its_open_parts() {
        let need = of(r"fn \w+_private\(", false);
        assert_eq!(need, all(vec![grams("fn "), grams("_private(")]));
        let need = of(r"fn \w+_unchecked\(", false);
        assert!(holds(&need, b"pub FN get_Unchecked(") && !holds(&need, b"fn get_checked("));
        assert_eq!(of(r"\w+\d*", false), Need::Any);
        assert_eq!(of("(needle)+", false), grams("needle"));
        assert_eq!(of("(needle)*", false), Need::Any);
        assert_eq!(of("ab?cde", false), grams("cde"));
    }

    #[test]
    fn alternatives_need_one_of_their_sides() {
        let need = of("TODO|FIXME", false);
        assert!(holds(&need, b"a todo here") && holds(&need, b"FixMe"));
        assert!(!holds(&need, b"nothing to do, fix me"));
        assert_eq!(of("needle|x", false), Need::Any);
        let classes = of("gr[ae]y", false);
        assert!(holds(&classes, b"grey") && holds(&classes, b"GRAY"));
        assert!(!holds(&classes, b"groy"));
    }

    #[test]
    fn a_line_end_is_never_part_of_a_need() {
        assert_eq!(gram(b"a\nb"), None);
        assert_eq!(gram(b"aBc"), gram(b"Abc"));
        assert_eq!(of("(?s)ab.cd", false), Need::Any);
        assert_eq!(of("[", false), Need::Any);
    }
}
