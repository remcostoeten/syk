use std::io::{self, IsTerminal, Write};
use std::process::ExitCode;
use std::time::Instant;

use crate::act;
use crate::group::Skip;
use crate::kept::{arrange, ranked, remembered, show};
use crate::pattern::Pattern;
use crate::pick;
use crate::plan::{Compiled, compile};
use crate::query::{Matching, Plan, Query, Targets};
use crate::rank::Habits;
use crate::replay::{Advice, background, reindex};
use crate::report::{counted, covered, grouped, several, totals};
use crate::search::{Outcome, Pass, View, search, view};
use crate::spell;
use crate::stop;
use crate::walk::excludes;
use crate::worker::Withheld;

const BIG: u64 = 20_000;

pub fn retryable(query: &Query) -> Option<(&str, Targets)> {
    match &query.plan {
        Plan::Union {
            pattern: Some(terms),
            targets,
        } => match terms.as_slice() {
            [source] if !query.output.exact && spell::correctable(source) => {
                Some((source, *targets))
            }
            _ => None,
        },
        _ => None,
    }
}

fn suggest(query: &Query) -> Option<String> {
    let (source, targets) = retryable(query)?;
    let matching = Matching {
        case: query.matching.case,
        literal: false,
        word: query.matching.word,
    };
    let pattern = Pattern::one(&spell::variants(source), &matching).ok()?;
    let fold = Pattern::one(source, &matching).ok()?.any_case(&matching);
    let plan = Compiled::Union {
        pattern: Some(pattern),
        targets,
    };
    let seen = search(query, plan, view(query), Pass::Gather { fold }, false).seen;
    spell::best(seen, source)
}

fn retry(query: &Query, view: View, keep: bool) -> Option<Outcome> {
    let (source, targets) = retryable(query)?;
    if !io::stderr().is_terminal() {
        return None;
    }
    let close = suggest(query)?;
    if !io::stdout().is_terminal() {
        eprintln!("syk: nothing matches {source:?}. Did you mean {close:?}?");
        return None;
    }
    let matching = Matching {
        case: query.matching.case,
        literal: true,
        word: query.matching.word,
    };
    let pattern = Pattern::one(&close, &matching).ok()?;
    eprintln!("syk: nothing matches {source:?}. Showing results for {close:?} instead.");
    let plan = Compiled::Union {
        pattern: Some(pattern),
        targets,
    };
    Some(search(query, plan, view, Pass::Again, keep))
}

pub fn run(query: Query) -> ExitCode {
    let started = Instant::now();
    if query.walk.index {
        return reindex(query, started);
    }
    stop::watch();
    let plan = match compile(&query) {
        Ok(plan) => plan,
        Err(error) => {
            eprintln!("syk: {error}");
            return ExitCode::from(2);
        }
    };
    if let Err(error) = excludes(&query) {
        eprintln!("syk: {error}");
        return ExitCode::from(2);
    }
    let view = view(&query);
    let rank = ranked(&query, view.live);
    let keep = rank || query.output.acts() || query.output.browse;
    let habits = rank.then(|| std::thread::spawn(Habits::load));
    let mut first = search(&query, plan, view, Pass::First, keep);
    let failed = first.failed;
    let large = first.advice == Advice::Large && first.tally.entries >= BIG;
    let build = (large || first.advice == Advice::Build) && !stop::asked();
    let habits = habits.and_then(|loading| loading.join().ok());
    if let Some(habits) = &habits {
        let extra = remembered(&query, habits, &first.blocks, view);
        first.found |= !extra.is_empty();
        first.blocks.extend(extra);
    }
    let last = match first.found || stop::asked() {
        true => first,
        false => retry(&query, view, keep).unwrap_or(first),
    };
    if stop::asked() && io::stderr().is_terminal() {
        eprint!("\r\x1b[K");
    }
    let found = last.found;
    let tally = last.tally;
    let reread = last.reread;
    let report = || {
        if query.output.stats {
            eprintln!(
                "syk: {}",
                totals(tally, view.layout, started.elapsed(), reread)
            );
        }
    };
    let mut ran = true;
    let chosen = query.output.acts() || query.output.best || query.output.browse;
    if keep && stop::asked() && chosen {
        report();
    } else if keep {
        let blocks = arrange(last.blocks, habits.as_ref(), &query);
        if query.output.browse && !blocks.is_empty() {
            report();
            let styled = std::env::var_os("NO_COLOR").is_none();
            let items = pick::items(blocks.iter().map(|(_, block)| block), styled);
            let item = match pick::choose(&items, styled) {
                Ok(Some(index)) => &items[index],
                Ok(None) => return ExitCode::from(1),
                Err(error) => {
                    eprintln!("syk: -b, --browse needs a terminal to show the list: {error}");
                    return ExitCode::from(2);
                }
            };
            if let Some(habits) = &habits {
                habits.remember(&item.path);
            }
            if query.output.open {
                let (program, error) = act::open(&item.path, item.line);
                eprintln!("syk: could not start {program}: {error}");
                return ExitCode::from(2);
            }
            match &query.output.exec {
                Some(command) => ran = act::execute(command, &[item.path.as_slice()]),
                None => {
                    let mut line = item.path.clone();
                    line.push(view.end);
                    let _ = io::stdout().lock().write_all(&line);
                }
            }
        } else if query.output.open {
            report();
            if let Some((_, top)) = blocks.first() {
                let (program, error) = act::open(&top.path, top.line);
                eprintln!("syk: could not start {program}: {error}");
                return ExitCode::from(2);
            }
        } else if let Some(command) = &query.output.exec {
            let paths: Vec<&[u8]> = blocks
                .iter()
                .map(|(_, block)| block.path.as_slice())
                .collect();
            ran = paths.is_empty() || act::execute(command, &paths);
            report();
        } else {
            show(blocks, &query, view);
            report();
        }
    } else {
        report();
    }

    if stop::asked() {
        eprintln!("syk: stopped before the search finished.");
        return ExitCode::from(130);
    }
    if build {
        background(&query);
    }
    if !found
        && io::stderr().is_terminal()
        && let Some(terms) = several(&query)
    {
        let words: Vec<String> = terms.iter().map(|term| format!("{term:?}")).collect();
        eprintln!(
            "syk: nothing has {} together. Every word has to be in the same name or on the same line. A word meant as a path has to exist, or end in a slash.",
            words.join(" and ")
        );
    }
    if !found
        && io::stderr().is_terminal()
        && let Some((group, name)) = covered(&query)
    {
        let group = &group.name;
        eprintln!(
            "syk: the group {group} is skipped, and {name} belongs to it. Add -u {group} to search it."
        );
    }
    let Withheld { files, lines } = tally.withheld;
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
    if found && ran {
        ExitCode::SUCCESS
    } else if found {
        ExitCode::from(1)
    } else if failed {
        ExitCode::from(2)
    } else {
        ExitCode::from(1)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::path::PathBuf;

    use super::*;
    use crate::parse::{self, Outcome as Parsed};

    struct Tree {
        root: PathBuf,
    }

    impl Tree {
        fn new(label: &str) -> Tree {
            let root =
                std::env::temp_dir().join(format!("syk-spell-{}-{label}", std::process::id()));
            for (path, text) in [
                ("src/config.ts", "export const config = loadConfig()\n"),
                ("src/notes.md", "confine the configuration\n"),
                ("settings/handler.rs", "fn handler() {}\n"),
            ] {
                let path = root.join(path);
                fs::create_dir_all(path.parent().expect("a parent folder")).expect("create");
                fs::write(path, text).expect("write");
            }
            Tree { root }
        }

        fn close(&self, args: &[&str]) -> Option<String> {
            let mut args: Vec<OsString> = args.iter().map(OsString::from).collect();
            args.push(format!("{}/", self.root.display()).into());
            match parse::parse(args) {
                Ok(Parsed::Run(query)) => suggest(&query),
                _ => panic!("expected a search"),
            }
        }
    }

    impl Drop for Tree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn a_missing_letter_is_found_in_names_and_contents() {
        let tree = Tree::new("missing");
        assert_eq!(tree.close(&["confg"]).as_deref(), Some("config"));
        assert_eq!(tree.close(&["-n", "handlr"]).as_deref(), Some("handler"));
        assert_eq!(tree.close(&["-d", "setings"]).as_deref(), Some("settings"));
    }

    #[test]
    fn extra_swapped_and_wrong_letters_are_found() {
        let tree = Tree::new("other");
        assert_eq!(tree.close(&["-c", "handlerr"]).as_deref(), Some("handler"));
        assert_eq!(tree.close(&["-c", "hadnler"]).as_deref(), Some("handler"));
        assert_eq!(tree.close(&["-c", "hendler"]).as_deref(), Some("handler"));
    }

    #[test]
    fn case_and_targets_are_respected() {
        let tree = Tree::new("case");
        assert_eq!(tree.close(&["Confg"]).as_deref(), Some("Config"));
        assert_eq!(tree.close(&["-d", "handlr"]), None);
        assert_eq!(tree.close(&["confg", ".rs"]), None);
    }

    #[test]
    fn nothing_close_or_switched_off_gives_no_suggestion() {
        let tree = Tree::new("none");
        assert_eq!(tree.close(&["zzzzqq"]), None);
        assert_eq!(tree.close(&["confg", "-X"]), None);
        assert_eq!(tree.close(&["conf.g"]), None);
    }
}
