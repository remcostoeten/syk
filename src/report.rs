use std::io::{self, IsTerminal};
use std::time::Duration;

use crate::group::{self, Group, Skip, Skips};
use crate::output::Layout;
use crate::query::{Plan, Query, Terms};
use crate::search::PROBE;
use crate::worker::{Tally, Withheld};

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
    let spared = match tally.spared {
        0 => String::new(),
        1 => " The index showed that 1 file could not match, so it was not read.".to_string(),
        files => format!(
            " The index showed that {} files could not match, so they were not read.",
            grouped(files)
        ),
    };
    format!(
        "{} matched. Read {} of {} in {time}.{source}{spared}",
        matched.join(", "),
        counted(tally.read, "file", "files"),
        counted(tally.entries, "entry", "entries")
    )
}

fn several(query: &Query) -> Option<&Terms> {
    let terms: Vec<&Option<Terms>> = match &query.plan {
        Plan::Union { pattern, .. } => vec![pattern],
        Plan::Narrow { dir, name, content } => vec![dir, name, content],
    };
    terms.into_iter().flatten().find(|terms| terms.len() > 1)
}

fn covered(query: &Query) -> Option<(&Group, &str)> {
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

#[derive(Clone, PartialEq, Eq)]
enum Left {
    Group(String),
    Hidden,
    Git,
    Ignored,
}

impl Left {
    fn told(&self, count: usize) -> String {
        let count = match count >= PROBE {
            true => format!("{PROBE} or more"),
            false => count.to_string(),
        };
        match self {
            Left::Group(name) => format!("{count} in the group {name} (add -u {name})"),
            Left::Hidden => format!("{count} in hidden files or folders (add -H)"),
            Left::Git => format!("{count} inside .git (add -a)"),
            Left::Ignored => format!("{count} that an ignore file leaves out (add -I)"),
        }
    }
}

fn below<'a>(query: &Query, path: &'a [u8]) -> &'a [u8] {
    let bare = |bytes: &'a [u8]| bytes.strip_prefix(b"./").unwrap_or(bytes);
    let inside = query.roots.iter().find_map(|root| {
        let root = root.as_os_str().as_encoded_bytes();
        let root = root.strip_prefix(b"./").unwrap_or(root);
        let root = root.strip_suffix(b"/").unwrap_or(root);
        let rest = bare(path).strip_prefix(root)?;
        (root != b"." && (rest.is_empty() || rest.starts_with(b"/"))).then_some(rest)
    });
    inside.unwrap_or(bare(path))
}

fn reason(query: &Query, groups: &[(String, Skips)], unread: &Skips, path: &[u8]) -> Option<Left> {
    let walk = &query.walk;
    let parts: Vec<&[u8]> = below(query, path)
        .split(|byte| *byte == b'/')
        .filter(|part| !part.is_empty())
        .collect();
    let grouped = |part: &&[u8]| groups.iter().find(|(_, skips)| skips.matches(part));
    if let Some((name, _)) = parts.iter().find_map(grouped) {
        return Some(Left::Group(name.clone()));
    }
    if !walk.hidden && parts.iter().any(|part| part.starts_with(b".")) {
        return Some(Left::Hidden);
    }
    if walk.hidden && parts.contains(&&b".git"[..]) {
        return Some(Left::Git);
    }
    let withheld = parts.last().is_some_and(|name| unread.matches(name));
    (!walk.no_ignore && !withheld).then_some(Left::Ignored)
}

fn left(query: &Query, paths: &[Vec<u8>]) -> Option<String> {
    let ruled = !query.walk.no_ignore;
    let whole = |group: &&Group| ruled && group.skip == Skip::Whole;
    let groups: Vec<(String, Skips)> = query
        .filters
        .groups
        .iter()
        .filter(whole)
        .map(|group| {
            let skips = Skips::new(std::slice::from_ref(group), Skip::Whole);
            (group.name.clone(), skips)
        })
        .collect();
    let unread = match ruled {
        true => Skips::new(&query.filters.groups, Skip::Contents),
        false => Skips::none(),
    };
    let mut counts: Vec<(Left, usize)> = Vec::new();
    for found in paths
        .iter()
        .filter_map(|path| reason(query, &groups, &unread, path))
    {
        match counts.iter_mut().find(|(known, _)| *known == found) {
            Some((_, count)) => *count += 1,
            None => counts.push((found, 1)),
        }
    }
    counts.sort_by_key(|(found, _)| match found {
        Left::Group(name) => (0, name.clone()),
        Left::Hidden => (1, String::new()),
        Left::Git => (2, String::new()),
        Left::Ignored => (3, String::new()),
    });
    let told: Vec<String> = counts
        .iter()
        .map(|(found, count)| found.told(*count))
        .collect();
    let every = match told.len() {
        0 => return None,
        1 => "",
        _ => " Add -a to search everything.",
    };
    Some(format!(
        "nothing matched, but places that are skipped have results: {}.{every}",
        told.join(", ")
    ))
}

pub fn hints(query: &Query, found: bool, withheld: Withheld, skipped: &[Vec<u8>]) {
    let left = left(query, skipped);
    if let Some(left) = &left {
        eprintln!("syk: {left}");
    }
    if !found
        && io::stderr().is_terminal()
        && let Some(terms) = several(query)
    {
        let words: Vec<String> = terms.iter().map(|term| format!("{term:?}")).collect();
        eprintln!(
            "syk: nothing has {} together. Every word has to be in the same name or on the same line. A word meant as a path has to exist, or end in a slash.",
            words.join(" and ")
        );
    }
    if !found
        && left.is_none()
        && io::stderr().is_terminal()
        && let Some((group, name)) = covered(query)
    {
        let group = &group.name;
        eprintln!(
            "syk: the group {group} is skipped, and {name} belongs to it. Add -u {group} to search it."
        );
    }
    let Withheld { files, lines } = withheld;
    let acting = query.output.best || query.output.browse || query.output.acts();
    if lines > 0 && !acting && (io::stderr().is_terminal() || query.output.stats) {
        let groups: Vec<&str> = query
            .filters
            .groups
            .iter()
            .filter(|group| group.skip == Skip::Contents)
            .map(|group| group.name.as_str())
            .collect();
        let matching = match (found, lines) {
            (true, 1) => "1 more line matches".to_string(),
            (true, _) => format!("{} more lines match", grouped(lines)),
            (false, 1) => "1 line matches".to_string(),
            (false, _) => format!("{} lines match", grouped(lines)),
        };
        eprintln!(
            "syk: {matching} in {} of the {} {}, which is not shown. Add -u {} to see {}.",
            counted(files, "file", "files"),
            if groups.len() == 1 { "group" } else { "groups" },
            groups.join(" and "),
            groups.join(","),
            if lines == 1 { "it" } else { "them" }
        );
    }
    if let Some(expanded) = &query.expanded
        && (expanded.files > 1 || !found)
        && io::stderr().is_terminal()
    {
        let extension = &expanded.extension;
        let searched = match expanded.files {
            1 => format!("only the one {extension} file in this folder was searched"),
            files => format!("only the {files} {extension} files in this folder were searched"),
        };
        eprintln!(
            "syk: {searched}. If you typed *{extension}, the shell replaced it with file names before syk ran. To search every {extension} file below this folder, write {extension} or '*{extension}'."
        );
    }
}
