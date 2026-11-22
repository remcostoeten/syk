use std::fmt::Write;

use crate::pattern::{Pattern, Reading, Term};
use crate::query::{Format, Kind, Matching, Query};
use crate::walk::{Compiled, compile};

fn row(out: &mut String, label: &str, value: &str) {
    let _ = writeln!(out, "{label:<10}{value}");
}

fn reading(term: &Term, any_case: bool, matching: &Matching) -> String {
    let kind = match term.reading {
        Reading::Regex => "a regular expression",
        Reading::Text if matching.literal => "plain text, because of -F",
        Reading::Text => "plain text, because it is not a valid regular expression",
        Reading::Glob => {
            "a wildcard on names and plain text in contents, because it is not a valid regular expression"
        }
    };
    let case = if any_case { "any case" } else { "exact case" };
    let word = if matching.word {
        ", whole words only"
    } else {
        ""
    };
    format!("read as {kind}, {case}{word}")
}

fn pattern(out: &mut String, label: &str, role: &str, pattern: &Pattern, matching: &Matching) {
    let any_case = pattern.any_case(matching);
    match pattern.terms() {
        [term] => {
            row(out, label, &format!("{:?}", term.source));
            row(out, "", &reading(term, any_case, matching));
        }
        terms => {
            for (index, term) in terms.iter().enumerate() {
                let label = if index == 0 { label } else { "and" };
                row(
                    out,
                    label,
                    &format!("{:?} {}", term.source, reading(term, any_case, matching)),
                );
            }
            row(
                out,
                "",
                "every one has to be in the same name or on the same line",
            );
        }
    }
    row(out, "", role);
}

pub fn render(query: &Query) -> Result<String, String> {
    let mut out = String::new();
    let matching = &query.matching;
    match compile(query)? {
        Compiled::Union {
            pattern: found,
            targets,
        } => {
            let mut places = Vec::new();
            if targets.dirs {
                places.push("folder names");
            }
            if targets.files {
                places.push("file names");
            }
            if targets.content {
                places.push("lines inside files");
            }
            match found {
                Some(found) => pattern(
                    &mut out,
                    "pattern",
                    &format!("matched against {}", places.join(", ")),
                    &found,
                    matching,
                ),
                None => row(
                    &mut out,
                    "pattern",
                    &format!("none, so this lists {}", places.join(" and ")),
                ),
            }
        }
        Compiled::Narrow { dir, name, content } => {
            if let Some(dir) = &dir {
                pattern(
                    &mut out,
                    "folders",
                    "only entries below a folder whose name matches are considered",
                    dir,
                    matching,
                );
            }
            if let Some(name) = &name {
                let role = if content.is_some() {
                    "only files whose name matches are read"
                } else {
                    "files and folders whose name matches are printed"
                };
                pattern(&mut out, "names", role, name, matching);
            }
            if let Some(content) = &content {
                pattern(
                    &mut out,
                    "contents",
                    "lines that match are printed",
                    content,
                    matching,
                );
            }
        }
    }

    let roots: Vec<String> = query
        .roots
        .iter()
        .map(|root| root.display().to_string())
        .collect();
    row(&mut out, "in", &roots.join(", "));

    let filters = &query.filters;
    let mut lines: Vec<String> = Vec::new();
    if !filters.extensions.is_empty() {
        lines.push(format!("name ends in {}", filters.extensions.join(" or ")));
    }
    for size in &filters.sizes {
        let side = if size.at_least { "at least" } else { "at most" };
        lines.push(format!("size {side} {} bytes ({})", size.bytes, size.text));
    }
    for age in &filters.ages {
        let side = if age.older {
            "more than"
        } else {
            "within the last"
        };
        let tail = if age.older { " ago" } else { "" };
        lines.push(format!(
            "modified {side} {} seconds{tail} ({})",
            age.seconds, age.text
        ));
    }
    match filters.kind {
        Some(Kind::File) => lines.push("regular files only".to_string()),
        Some(Kind::Dir) => lines.push("folders only".to_string()),
        Some(Kind::Link) => lines.push("symbolic links only".to_string()),
        None => {}
    }
    if !filters.excludes.is_empty() {
        lines.push(format!("leaves out {}", filters.excludes.join(", ")));
    }
    if let Some(depth) = query.walk.depth {
        lines.push(format!("at most {depth} levels deep"));
    }
    if filters.files_only() {
        lines.push("folders are not reported while a file filter is active".to_string());
    }
    for (index, line) in lines.iter().enumerate() {
        row(&mut out, if index == 0 { "filters" } else { "" }, line);
    }

    let walk = &query.walk;
    let hidden = walk.hidden || walk.all;
    let ignored = walk.no_ignore || walk.all;
    let skipped: Vec<&str> = [
        (!hidden, "hidden entries"),
        (!ignored, "ignored entries"),
        (hidden && !walk.all, "the inside of .git"),
        (true, "binary file contents"),
    ]
    .into_iter()
    .filter(|(skips, _)| *skips)
    .map(|(_, what)| what)
    .collect();
    row(&mut out, "skips", &skipped.join(", "));
    for group in filters
        .groups
        .iter()
        .filter(|group| group.active && !ignored)
    {
        let names = group.patterns.join(", ");
        row(&mut out, "", &format!("the group {}: {names}", group.name));
    }

    let mut output: Vec<String> = Vec::new();
    if query.output.browse {
        let then = match (&query.output.exec, query.output.open) {
            (Some(command), _) => format!("runs {command:?} on it"),
            (None, true) => "opens it in $VISUAL or $EDITOR".to_string(),
            (None, false) => "prints its path".to_string(),
        };
        output.push(format!(
            "shows a list to choose one result from, then {then}"
        ));
    } else if let Some(command) = &query.output.exec {
        let how = if command.contains("{}") {
            "once per result"
        } else {
            "once, with every result"
        };
        output.push(format!("runs {command:?} {how}"));
    } else if query.output.open {
        output.push("opens the best result in $VISUAL or $EDITOR".to_string());
    } else if query.output.null {
        output.push("paths only, separated by NUL bytes".to_string());
    } else {
        if query.output.list {
            output.push("paths only".to_string());
        } else if query.output.count {
            output.push("matching lines counted per file".to_string());
        }
        match query.output.format {
            _ if query.output.json => output.push("one JSON object per line".to_string()),
            Format::Plain => output.push("plain".to_string()),
            Format::Pretty => output.push("pretty".to_string()),
            Format::Auto => output.push("pretty in a terminal, plain in a pipe".to_string()),
        }
    }
    if query.output.stats {
        output.push("totals at the end".to_string());
    }
    if let Some(max) = query.output.max {
        output.push(format!("stops after {max} results"));
    }
    if query.output.context > 0 {
        output.push(format!("{} lines of context", query.output.context));
    }
    if crate::walk::retryable(query).is_some() {
        output.push("retries with a close spelling when nothing is found".to_string());
    }
    if crate::walk::ranked(query, std::io::IsTerminal::is_terminal(&std::io::stdout())) {
        output.push("ranked by relevance".to_string());
    }
    if walk.sort {
        output.push("sorted by path, one thread".to_string());
    }
    row(&mut out, "output", &output.join(", "));
    if let Some(defaults) = &query.defaults {
        row(&mut out, "defaults", &defaults.args.join(" "));
        row(&mut out, "", &format!("from {}", defaults.path.display()));
    }
    Ok(out)
}
