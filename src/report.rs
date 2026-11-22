use std::time::Duration;

use crate::group::{self, Group};
use crate::output::Layout;
use crate::query::{Plan, Query, Terms};
use crate::worker::Tally;

pub fn grouped(number: u64) -> String {
    let digits = number.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn counted(number: u64, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        grouped(number),
        if number == 1 { one } else { many }
    )
}

pub fn totals(tally: Tally, layout: Layout, elapsed: Duration, reread: Option<usize>) -> String {
    let mut matched: Vec<String> = Vec::new();
    if layout != Layout::List && tally.files > 0 {
        matched.push(format!(
            "{} in {}",
            counted(tally.lines, "line", "lines"),
            counted(tally.files, "file", "files")
        ));
    } else if tally.files > 0 {
        matched.push(counted(tally.files, "file", "files"));
    }
    if tally.names > 0 {
        matched.push(counted(tally.names, "name", "names"));
    }
    if matched.is_empty() {
        matched.push("nothing".to_string());
    }
    let time = match elapsed.as_millis() {
        millis if millis < 1000 => format!("{millis} ms"),
        _ => format!("{:.2} s", elapsed.as_secs_f64()),
    };
    let source = match reread {
        None => String::new(),
        Some(0) => " The names came from the index.".to_string(),
        Some(folders) => format!(
            " The names came from the index, and {} read again.",
            match folders {
                1 => "1 changed folder was".to_string(),
                _ => format!("{} changed folders were", grouped(folders as u64)),
            }
        ),
    };
    format!(
        "{} matched. Read {} of {} in {time}.{source}",
        matched.join(", "),
        counted(tally.read, "file", "files"),
        counted(tally.entries, "entry", "entries")
    )
}

pub fn several(query: &Query) -> Option<&Terms> {
    let terms: Vec<&Option<Terms>> = match &query.plan {
        Plan::Union { pattern, .. } => vec![pattern],
        Plan::Narrow { dir, name, content } => vec![dir, name, content],
    };
    terms.into_iter().flatten().find(|terms| terms.len() > 1)
}

pub fn covered(query: &Query) -> Option<(&Group, &str)> {
    if query.walk.no_ignore || query.walk.all {
        return None;
    }
    let names: Vec<&Option<Terms>> = match &query.plan {
        Plan::Union { pattern, targets } if targets.dirs || targets.files => vec![pattern],
        Plan::Union { .. } => Vec::new(),
        Plan::Narrow { dir, name, .. } => vec![dir, name],
    };
    names
        .into_iter()
        .flatten()
        .flatten()
        .find_map(|term| group::covering(&query.filters.groups, term))
}
