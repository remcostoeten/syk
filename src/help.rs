use std::io::{IsTerminal, Write};

use crate::banner::{self, Look, TAGLINE};
use crate::filter;
use crate::flags::{self, Arity, FLAGS, Flag, Group, Id};
use crate::pager::{self, Mark, Setup, Sheet};

const MARGIN: &str = "  ";
const INDENT: &str = "    ";
const GAP: usize = 3;
const ROOM: usize = 24;
const NARROWEST: usize = 40;
const QUOTED: usize = 3;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const PATTERN: &str = "\x1b[1;31m";
const PATH: &str = "\x1b[1;34m";
const FILTER: &str = "\x1b[33m";
const FLAG: &str = "\x1b[36m";
const NONE: &str = "";
const RULE: &str = "─";

const SHAPES: [(&str, &str, &str); 9] = [
    ("foo", "pattern", "the text to search for"),
    (
        "src/",
        "path",
        "where to search, when it exists on disk. Default: here",
    ),
    (
        ".ts",
        "extension",
        "only files ending in .ts. '*.ts' means the same",
    ),
    ("+10kb", "size", "only files of 10 KB or more"),
    ("-1mb", "size", "only files of 1 MB or less"),
    ("+7d", "age", "only files modified more than 7 days ago"),
    ("-2h", "age", "only files modified in the last 2 hours"),
    ("-", "pipe", "search what is piped in, not files"),
    (
        "--",
        "separator",
        "what follows is literal: the pattern, then paths",
    ),
];

const SHAPES_LEAD: &str = "Words that are not flags are recognised by their shape.";

const SHAPES_NOTE: &str = "A word that could be a pattern and also exists on disk is read as the pattern when it is the only candidate. End it with a slash to make it a path: src/.";

const PATTERNS: [(&str, &str); 7] = [
    ("foo", "lower case matches any case: foo, Foo and FOO"),
    (
        "foo bar",
        "both words, in the same name or on the same line",
    ),
    ("Foo", "one upper case letter makes the case exact"),
    ("^foo\\d+", "a regular expression"),
    ("a(b", "not a valid one, so searched as plain text"),
    ("'*conf*'", "a wildcard on names, plain text in contents"),
    (
        "confg",
        "nothing found: a spelling one edit away is tried, config",
    ),
];

const EXAMPLES: [(&str, &str); 16] = [
    ("syk foo", "folders, files and lines matching foo"),
    (
        "syk foo src/ .ts",
        "the same, in TypeScript files below src/",
    ),
    ("syk -n config", "names only"),
    ("syk -d test", "folder names only"),
    ("syk -c TODO -l", "paths of the files that contain TODO"),
    (
        "syk -n route -c auth",
        "lines matching auth in files named *route*",
    ),
    (
        "syk -d api -c auth",
        "lines matching auth below folders named *api*",
    ),
    ("cat app.log | syk error -", "search what is piped in"),
    ("syk foo -E dist,build", "leave out two folders"),
    (
        "syk .log +30d +1mb",
        "log files older than 30 days and over 1 MB",
    ),
    ("syk -2h", "everything modified in the last 2 hours"),
    ("syk -c TODO -N -T", "TODO lines per file, then the totals"),
    ("syk -c 'fn main' -O", "open the best hit in your editor"),
    (
        "syk .log +30d -f 'gzip {}'",
        "run a command on every result",
    ),
    ("cd (syk -d dora -1)", "jump to the best matching folder"),
    ("syk 'use client' .tsx -x", "show how this would be read"),
];

const EXIT: [(&str, &str); 3] = [
    ("0", "something was found"),
    ("1", "nothing was found"),
    ("2", "an argument was wrong or something was unreadable"),
];

const MORE: [(&str, &str); 6] = [
    ("syk -n -h", "the page for one flag"),
    ("syk --size --help", "the same, with the long names"),
    ("syk -h modified", "the same, by name"),
    ("syk foo -x", "show how the arguments are read"),
    ("syk -h pipe", "the flags whose page mentions pipe"),
    ("syk -h -h", "the paging keys, and how to turn paging off"),
];

fn lead(group: Group) -> Option<&'static str> {
    match group {
        Group::Target => {
            Some("Without these, the pattern is tried on all three and every hit is shown.")
        }
        Group::Filter => {
            Some("-e, -S and -M also have the shorthand from ARGUMENTS: .ts, +10kb, -2h.")
        }
        Group::Walk => Some(
            "Hidden, ignored and binary files are skipped unless you ask for them, and so are dependency folders and caches. Lock files are found by name, but their contents are not shown.",
        ),
        Group::Output => Some(
            "In a terminal, results are grouped and coloured. In a pipe they are plain, one per line.",
        ),
        Group::Ranking => Some(
            "Ranked results end with the best one, next to your prompt. In a pipe the best one comes first. A search on names only is ranked by default in a terminal.",
        ),
        _ => None,
    }
}

fn note(group: Group) -> Option<&'static str> {
    match group {
        Group::Target => Some(
            "Two of them with their own pattern narrow each other: -n route -c auth prints the lines matching auth in files named *route*.",
        ),
        Group::Walk => Some(
            "The groups are deps, cache, build, lock, min and media. syk -g -h lists the names in each one.",
        ),
        Group::Other => Some(
            "Flags you always want go in ~/.config/syk/config, written as you would type them. syk -U -h has the details.",
        ),
        _ => None,
    }
}

fn put(out: &mut String, style: &str, text: &str, styled: bool) {
    if styled && !style.is_empty() && !text.is_empty() {
        out.push_str(style);
        out.push_str(text);
        out.push_str(RESET);
    } else {
        out.push_str(text);
    }
}

fn tone(word: &str) -> &'static str {
    let bare = word.trim_matches('\'');
    if word == "syk" {
        DIM
    } else if filter::is_token(word) || bare.starts_with('.') || bare.starts_with("*.") {
        FILTER
    } else if word.starts_with('-') {
        FLAG
    } else if word.ends_with('/') {
        PATH
    } else {
        PATTERN
    }
}

fn named(word: &str) -> Option<&'static Flag> {
    match word.strip_prefix("--") {
        Some(long) => flags::by_long(long),
        None => flags::by_short(word.chars().last()?),
    }
}

fn value(flag: &Flag) -> &'static str {
    if flag.group == Group::Target {
        PATTERN
    } else {
        FILTER
    }
}

fn paint(out: &mut String, command: &str, styled: bool) {
    let nested = command.contains("(syk");
    let mut started = !command.contains("syk");
    let mut piped = false;
    let mut before: Option<&'static Flag> = None;
    let mut quoted: Option<&'static str> = None;
    for (index, raw) in command.split(' ').enumerate() {
        if index > 0 {
            out.push(' ');
        }
        piped = piped || raw == "|";
        if piped {
            out.push_str(raw);
            continue;
        }
        let mut word = raw;
        let mut closing = "";
        if nested && let Some(rest) = word.strip_prefix('(') {
            out.push('(');
            word = rest;
        }
        if nested && let Some(rest) = word.strip_suffix(')') {
            closing = ")";
            word = rest;
        }
        if !started {
            started = word == "syk";
            put(out, if started { DIM } else { NONE }, word, styled);
        } else if let Some(style) = quoted {
            put(out, style, word, styled);
            if word.ends_with('\'') {
                quoted = None;
            }
        } else if word.len() > 2
            && word.starts_with('-')
            && let Some((name, attached)) = word.split_once('=')
        {
            put(out, FLAG, name, styled);
            out.push('=');
            put(out, named(name).map_or(PATTERN, value), attached, styled);
            before = None;
        } else {
            let own = tone(word);
            let style = match before {
                Some(flag) if matches!(flag.arity, Arity::Required(_)) => value(flag),
                Some(flag) if flag.id == Id::Help && own == PATTERN => FLAG,
                _ => own,
            };
            put(out, style, word, styled);
            if word.starts_with('\'') && !(word.len() > 1 && word.ends_with('\'')) {
                quoted = Some(style);
            }
            before = (style == FLAG && own == FLAG)
                .then(|| named(word))
                .flatten();
        }
        out.push_str(closing);
    }
}

fn plain(out: &mut String, text: &str, _styled: bool) {
    out.push_str(text);
}

fn label(out: &mut String, flag: &Flag, styled: bool) {
    put(out, FLAG, &format!("-{}", flag.short), styled);
    put(out, DIM, ", ", styled);
    put(out, FLAG, &format!("--{}", flag.long), styled);
    match flag.arity {
        Arity::Switch => {}
        Arity::Optional(name) => put(out, DIM, &format!(" [{name}]"), styled),
        Arity::Required(name) => put(out, DIM, &format!(" {name}"), styled),
    }
}

fn usage(out: &mut String, styled: bool) {
    out.push_str(INDENT);
    put(out, DIM, "syk", styled);
    for (style, word) in [
        (PATTERN, "pattern"),
        (PATH, "path/"),
        (FILTER, "filters"),
        (FLAG, "-flags"),
    ] {
        put(out, DIM, " [", styled);
        put(out, style, word, styled);
        put(out, DIM, "]", styled);
    }
    out.push('\n');
    out.push_str(INDENT);
    put(out, DIM, "in any order, all optional", styled);
    out.push('\n');
}

fn title(out: &mut String, look: Look) {
    if look.logo {
        out.push_str(&banner::header(TAGLINE, look));
        return;
    }
    let styled = look.styled;
    out.push('\n');
    out.push_str(MARGIN);
    put(out, BOLD, "syk", styled);
    out.push(' ');
    put(out, DIM, env!("CARGO_PKG_VERSION"), styled);
    out.push('\n');
    out.push_str(MARGIN);
    out.push_str(TAGLINE);
    out.push('\n');
}

fn wrap(out: &mut String, text: &str, start: usize, hang: usize, width: usize) {
    let mut column = start;
    let mut empty = true;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if !empty && column + 1 + length > width {
            out.push('\n');
            out.push_str(&" ".repeat(hang));
            column = hang;
            empty = true;
        }
        if !empty {
            out.push(' ');
            column += 1;
        }
        out.push_str(word);
        column += length;
        empty = false;
    }
    out.push('\n');
}

fn widest(flags: &[&Flag]) -> usize {
    flags
        .iter()
        .map(|flag| flag.label().chars().count())
        .max()
        .unwrap_or(0)
}

struct Page {
    text: String,
    marks: Vec<Mark>,
    look: Look,
    styled: bool,
    width: usize,
}

impl Page {
    fn new(look: Look) -> Page {
        Page {
            text: String::with_capacity(8192),
            marks: Vec::new(),
            look,
            styled: look.styled,
            width: look.columns.max(NARROWEST),
        }
    }

    fn mark(&mut self, title: &str) {
        self.marks.push(Mark {
            line: self.text.matches('\n').count(),
            title: title.to_string(),
        });
    }

    fn heading(&mut self, title: &str) {
        self.text.push('\n');
        self.mark(title);
        self.text.push_str(MARGIN);
        put(&mut self.text, BOLD, title, self.styled);
        let used = 2 * MARGIN.len() + title.chars().count() + 1;
        if self.look.banner && self.look.columns > used {
            self.text.push(' ');
            let rule = RULE.repeat(self.look.columns - used);
            put(&mut self.text, DIM, &rule, self.styled);
        }
        self.text.push('\n');
    }

    fn shapes(&mut self) {
        let first = SHAPES.iter().map(|row| row.0.len()).max().unwrap_or(0);
        let second = SHAPES.iter().map(|row| row.1.len()).max().unwrap_or(0);
        for (shape, kind, meaning) in SHAPES {
            let style = tone(shape);
            self.text.push_str(INDENT);
            put(&mut self.text, style, shape, self.styled);
            self.text.push_str(&" ".repeat(first - shape.len() + GAP));
            put(&mut self.text, style, kind, self.styled);
            self.meaning(first + GAP + kind.len(), first + GAP + second, meaning);
        }
    }

    fn paragraph(&mut self, text: &str) {
        self.text.push_str(INDENT);
        wrap(&mut self.text, text, INDENT.len(), INDENT.len(), self.width);
    }

    fn gap(&mut self) {
        self.text.push('\n');
    }

    fn meaning(&mut self, width: usize, column: usize, text: &str) {
        let start = INDENT.len() + column + GAP;
        if start + ROOM > self.width {
            let hang = 2 * INDENT.len();
            self.text.push('\n');
            self.text.push_str(&" ".repeat(hang));
            wrap(&mut self.text, text, hang, hang, self.width);
            return;
        }
        self.text
            .push_str(&" ".repeat(start - INDENT.len() - width));
        wrap(&mut self.text, text, start, start, self.width);
    }

    fn table(&mut self, rows: &[(&str, &str)], left: fn(&mut String, &str, bool)) {
        let column = rows
            .iter()
            .map(|(text, _)| text.chars().count())
            .max()
            .unwrap_or(0);
        for (text, meaning) in rows {
            self.text.push_str(INDENT);
            left(&mut self.text, text, self.styled);
            self.meaning(text.chars().count(), column, meaning);
        }
    }

    fn flags(&mut self, flags: &[&Flag], column: usize) {
        for flag in flags {
            self.text.push_str(INDENT);
            label(&mut self.text, flag, self.styled);
            self.meaning(flag.label().chars().count(), column, flag.summary);
        }
    }
}

fn overview(look: Look) -> Page {
    let styled = look.styled;
    let mut page = Page::new(look);
    title(&mut page.text, look);

    page.heading("USAGE");
    usage(&mut page.text, styled);

    page.heading("ARGUMENTS");
    page.paragraph(SHAPES_LEAD);
    page.gap();
    page.shapes();
    page.gap();
    page.paragraph(SHAPES_NOTE);

    page.heading("PATTERNS");
    page.table(&PATTERNS, paint);

    let every: Vec<&Flag> = FLAGS.iter().collect();
    let column = widest(&every);
    for group in Group::ALL {
        let members: Vec<&Flag> = FLAGS.iter().filter(|flag| flag.group == group).collect();
        page.heading(group.title());
        if let Some(text) = lead(group) {
            page.paragraph(text);
            page.gap();
        }
        page.flags(&members, column);
        if let Some(text) = note(group) {
            page.gap();
            page.paragraph(text);
        }
    }

    page.heading("EXAMPLES");
    page.table(&EXAMPLES, paint);

    page.heading("EXIT STATUS");
    page.table(&EXIT, plain);

    page.heading("MORE HELP");
    page.table(&MORE, paint);
    page.gap();
    page
}

fn topic(page: &mut Page, flag: &Flag) {
    page.text.push('\n');
    page.mark(&format!("--{}", flag.long));
    page.text.push_str(MARGIN);
    label(&mut page.text, flag, page.styled);
    page.text.push('\n');
    page.paragraph(flag.summary);
    for text in flag.detail.split("\n\n") {
        page.gap();
        page.paragraph(text);
    }
    if !flag.aliases.is_empty() {
        let names: Vec<String> = flag
            .aliases
            .iter()
            .map(|alias| format!("--{alias}"))
            .collect();
        page.gap();
        page.paragraph(&format!("Also accepted: {}.", names.join(", ")));
    }
    if !flag.examples.is_empty() {
        page.heading("EXAMPLES");
        page.table(flag.examples, paint);
    }
    if !flag.related.is_empty() {
        let related: Vec<&Flag> = flag.related.iter().map(|id| flags::by_id(*id)).collect();
        page.heading("SEE ALSO");
        page.flags(&related, widest(&related));
    }
}

fn about(topics: &[Id]) -> String {
    match topics {
        [id] => format!("Help for --{}.", flags::by_id(*id).long),
        _ => format!("Help for {} flags.", topics.len()),
    }
}

fn mentions(flag: &Flag, term: &str) -> bool {
    let within = |text: &str| text.to_lowercase().contains(term);
    within(flag.long)
        || within(flag.summary)
        || within(flag.detail)
        || flag.aliases.iter().any(|alias| within(alias))
        || flag
            .examples
            .iter()
            .any(|(command, meaning)| within(command) || within(meaning))
}

fn sentences<'a>(detail: &'a str, terms: &[String]) -> Vec<&'a str> {
    detail
        .split("\n\n")
        .flat_map(|paragraph| paragraph.split_inclusive(". "))
        .map(str::trim)
        .filter(|sentence| {
            let lower = sentence.to_lowercase();
            terms.iter().any(|term| lower.contains(term))
        })
        .take(QUOTED)
        .collect()
}

fn found(terms: &[String], look: Look) -> Page {
    let shown = terms.join(" ");
    let lower: Vec<String> = terms.iter().map(|term| term.to_lowercase()).collect();
    let hits: Vec<&Flag> = FLAGS
        .iter()
        .filter(|flag| lower.iter().all(|term| mentions(flag, term)))
        .collect();
    let mut page = Page::new(look);
    let about = format!("Flags whose page mentions {shown}.");
    page.text.push_str(&banner::header(&about, look));
    if hits.is_empty() {
        page.gap();
        page.text.push_str(MARGIN);
        wrap(
            &mut page.text,
            &format!("No flag page mentions {shown}."),
            MARGIN.len(),
            MARGIN.len(),
            page.width,
        );
    }
    for flag in &hits {
        page.text.push('\n');
        page.mark(&format!("--{}", flag.long));
        page.text.push_str(MARGIN);
        label(&mut page.text, flag, page.styled);
        page.text.push('\n');
        page.paragraph(flag.summary);
        let quoted = sentences(flag.detail, &lower);
        if !quoted.is_empty() {
            page.gap();
            page.paragraph(&quoted.join(" "));
        }
        let rows: Vec<(&str, &str)> = flag
            .examples
            .iter()
            .filter(|(command, meaning)| {
                let text = format!("{command} {meaning}").to_lowercase();
                lower.iter().any(|term| text.contains(term))
            })
            .copied()
            .collect();
        if !rows.is_empty() {
            page.gap();
            page.table(&rows, paint);
        }
    }
    page.heading("MORE HELP");
    let first = hits.first().map(|flag| format!("syk -{} -h", flag.short));
    let mut rows: Vec<(&str, &str)> = Vec::with_capacity(2);
    if let Some(command) = &first {
        rows.push((command, "the full page of one flag"));
    }
    rows.push(("syk -h", "every flag"));
    page.table(&rows, paint);
    page.gap();
    page
}

fn build(topics: &[Id], terms: &[String], look: Look) -> Page {
    if topics.is_empty() && !terms.is_empty() {
        return found(terms, look);
    }
    if topics.is_empty() {
        return overview(look);
    }
    let mut page = Page::new(look);
    page.text.push_str(&banner::header(&about(topics), look));
    for id in topics {
        topic(&mut page, flags::by_id(*id));
    }
    page.heading("MORE HELP");
    page.table(&[("syk -h", "every flag")], paint);
    page.gap();
    page
}

#[cfg(test)]
pub fn render(topics: &[Id], look: Look) -> String {
    build(topics, &[], look).text
}

const WELCOME: [(&str, &[(&str, &str)]); 4] = [
    (
        "SEARCH",
        &[
            ("syk foo", "folders, files and lines matching foo"),
            ("syk -n foo", "names only"),
            ("syk -d foo", "folder names only"),
            ("syk -c foo", "contents only"),
            ("syk foo bar", "both words, in one name or on one line"),
            (
                "syk -n route -c auth",
                "lines matching auth in files named *route*",
            ),
        ],
    ),
    (
        "NARROW IT DOWN",
        &[
            ("syk foo src/", "below a folder"),
            ("syk foo .ts", "by extension"),
            (
                "syk foo +10kb -1mb",
                "by size: at least 10 KB, at most 1 MB",
            ),
            ("syk foo +7d", "by age: modified over 7 days ago"),
            ("syk foo -2h", "by age: modified in the last 2 hours"),
            ("syk foo -E dist", "leave out anything called dist"),
        ],
    ),
    (
        "USE THE RESULTS",
        &[
            ("syk foo -O", "open the best hit in your editor"),
            ("syk .log +30d -f gzip", "run a command on the results"),
            ("syk -c foo -N", "count the matching lines per file"),
            ("syk foo -J", "one JSON object per result"),
        ],
    ),
    (
        "LEARN MORE",
        &[
            ("syk -h", "every flag, in its short and long form"),
            ("syk -n -h", "the page for one flag"),
            ("syk foo -x", "show how the arguments are read"),
            ("syk --setup", "add tab completion and the s jump command"),
        ],
    ),
];

fn only_flag(command: &str) -> Option<&'static Flag> {
    let mut shorts = command
        .split(' ')
        .filter(|word| tone(word) == FLAG)
        .filter_map(|word| word.strip_prefix('-'));
    let short = shorts.next().filter(|_| shorts.next().is_none())?;
    flags::by_short(short.chars().next()?)
}

fn greeting(look: Look) -> Page {
    let styled = look.styled;
    let mut page = Page::new(look);
    title(&mut page.text, look);
    page.heading("USAGE");
    usage(&mut page.text, styled);

    let column = WELCOME
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(command, _)| command.len())
        .max()
        .unwrap_or(0);
    for (heading, rows) in WELCOME {
        page.heading(heading);
        let out = &mut page.text;
        for (command, meaning) in rows {
            out.push_str(INDENT);
            paint(out, command, styled);
            out.push_str(&" ".repeat(column - command.len() + GAP));
            out.push_str(meaning);
            if let Some(flag) = only_flag(command) {
                out.push_str("  ");
                put(out, DIM, &format!("or --{}", flag.long), styled);
            }
            out.push('\n');
        }
    }
    page.gap();
    page
}

#[cfg(test)]
pub fn welcome(look: Look) -> String {
    greeting(look).text
}

fn sheet(page: Page) -> Sheet {
    Sheet {
        text: page.text,
        marks: page.marks,
    }
}

fn paged(page: Page, terminal: bool) -> Result<(), String> {
    let look = page.look;
    let first = sheet(page);
    if !terminal {
        return Err(first.text);
    }
    let open = |short: char| flags::by_short(short).map(|flag| sheet(build(&[flag.id], &[], look)));
    let setup = Setup {
        styled: look.styled,
        open: Some(&open),
    };
    pager::offer(first, &setup).map_err(|sheet| sheet.text)
}

pub fn print_welcome() {
    let stderr = std::io::stderr();
    let look = Look::of(&stderr);
    let terminal = look.banner && std::io::stdout().is_terminal();
    if let Err(text) = paged(greeting(look), terminal) {
        let _ = stderr.lock().write_all(text.as_bytes());
    }
}

pub fn print(topics: &[Id], terms: &[String]) {
    let stdout = std::io::stdout();
    let look = Look::of(&stdout);
    if let Err(text) = paged(build(topics, terms, look), look.banner) {
        let _ = stdout.lock().write_all(text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::banner::WIDTH;

    const PIPED: Look = Look {
        banner: false,
        logo: false,
        styled: false,
        columns: WIDTH,
    };
    const SHOWN: Look = Look {
        banner: true,
        logo: true,
        styled: false,
        columns: WIDTH,
    };
    const STYLED: Look = Look {
        banner: true,
        logo: true,
        styled: true,
        columns: WIDTH,
    };

    #[test]
    fn every_page_starts_with_the_same_header() {
        let logo = banner::header(TAGLINE, SHOWN);
        let art: Vec<&str> = logo.lines().take(3).collect();
        let pages = [
            welcome(SHOWN),
            render(&[], SHOWN),
            render(&[Id::Name], SHOWN),
            render(&[Id::Name, Id::Content], SHOWN),
        ];
        for page in &pages {
            assert_eq!(page.lines().take(3).collect::<Vec<&str>>(), art);
            assert!(page.lines().all(|line| line.chars().count() <= WIDTH));
        }
        assert!(pages[0].contains(TAGLINE));
        assert!(pages[2].contains("Help for --name."));
        assert!(pages[3].contains("Help for 2 flags."));
        assert!(!render(&[Id::Name], PIPED).contains("Help for"));
    }

    #[test]
    fn a_terminal_gets_ruled_headings_and_marks_for_the_pager() {
        let page = build(&[], &[], SHOWN);
        let lines: Vec<&str> = page.text.lines().collect();
        assert!(
            page.text
                .contains(&format!("  FILTERS {}\n", RULE.repeat(68)))
        );
        assert!(!render(&[], PIPED).contains(RULE));
        assert_eq!(page.marks.len(), 14);
        for mark in &page.marks {
            assert!(lines[mark.line].starts_with(&format!("  {} ", mark.title)));
            assert_eq!(lines[mark.line - 1], "");
        }
        let size = build(&[Id::Size], &[], SHOWN);
        assert_eq!(size.marks[0].title, "--size");
        assert!(size.text.lines().nth(size.marks[0].line) == Some("  -S, --size SIZE"));
    }

    #[test]
    fn a_narrow_terminal_gets_narrow_pages() {
        let narrow = Look {
            banner: true,
            logo: true,
            styled: false,
            columns: 48,
        };
        let mut pages = vec![welcome(narrow), render(&[], narrow)];
        pages.extend(FLAGS.iter().map(|flag| render(&[flag.id], narrow)));
        for page in pages.iter().skip(1) {
            let wide = page
                .lines()
                .find(|line| line.chars().count() > 48 && line.trim().contains(' '));
            assert_eq!(wide, None);
        }
        assert!(
            pages[1]
                .contains("    -c, --content [PATTERN]\n        Search the lines inside files.\n")
        );
    }

    #[test]
    fn welcome_is_plain_without_a_terminal() {
        let plain = welcome(PIPED);
        assert!(!plain.contains('\x1b'));
        assert!(plain.contains("syk -n route -c auth    lines matching auth"));
        assert!(plain.contains("folder names only  or --dir\n"));
        assert!(plain.contains("names only  or --name\n"));
        assert!(plain.lines().all(|line| line.chars().count() <= 80));
    }

    #[test]
    fn overview_keeps_every_flag_on_one_line() {
        let plain = render(&[], PIPED);
        assert!(!plain.contains('\x1b'));
        assert!(plain.lines().all(|line| line.chars().count() <= WIDTH));
        for flag in &FLAGS {
            let row = format!("{}{}", INDENT, flag.label());
            let line = plain
                .lines()
                .find(|line| line.starts_with(&row))
                .unwrap_or_else(|| panic!("no row for --{}", flag.long));
            assert!(line.ends_with(flag.summary), "--{} wraps", flag.long);
        }
    }

    #[test]
    fn pages_fit_the_width_and_paint_their_examples() {
        for flag in &FLAGS {
            let plain = render(&[flag.id], PIPED);
            assert!(!plain.contains('\x1b'));
            assert!(plain.lines().all(|line| line.chars().count() <= WIDTH));
        }
        let styled = render(&[Id::Exclude], STYLED);
        for painted in [
            format!("{FLAG}-E{RESET} {FILTER}dist{RESET}"),
            format!("{FLAG}-c{RESET} {PATTERN}TODO{RESET}"),
            format!("{FLAG}--exclude{RESET}{DIM} NAME{RESET}"),
        ] {
            assert!(styled.contains(&painted), "{painted:?} is missing");
        }
    }

    #[test]
    fn a_quoted_value_keeps_one_colour() {
        let mut out = String::new();
        paint(&mut out, "syk .log -f 'gzip -9 {}' -m 3", true);
        assert_eq!(
            out,
            format!(
                "{DIM}syk{RESET} {FILTER}.log{RESET} {FLAG}-f{RESET} {FILTER}'gzip{RESET} {FILTER}-9{RESET} {FILTER}{{}}'{RESET} {FLAG}-m{RESET} {FILTER}3{RESET}"
            )
        );
    }

    #[test]
    fn commands_outside_syk_stay_plain() {
        let mut out = String::new();
        paint(&mut out, "cd (syk -d dora -1) | less -R", true);
        assert_eq!(
            out,
            format!(
                "cd ({DIM}syk{RESET} {FLAG}-d{RESET} {PATTERN}dora{RESET} {FLAG}-1{RESET}) | less -R"
            )
        );
    }

    #[test]
    fn welcome_colours_each_kind_of_argument() {
        let styled = welcome(STYLED);
        for (style, word) in [
            (PATTERN, "foo"),
            (PATH, "src/"),
            (FILTER, "+10kb"),
            (FILTER, ".ts"),
            (FLAG, "-n"),
        ] {
            assert!(
                styled.contains(&format!("{style}{word}{RESET}")),
                "{word} is not styled"
            );
        }
    }
}
