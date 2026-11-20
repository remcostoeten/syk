use std::io::Write;

use crate::banner::{self, Look, TAGLINE};
use crate::filter;
use crate::flags::{self, Arity, FLAGS, Flag, Group, Id};

const WIDTH: usize = 80;
const MARGIN: &str = "  ";
const INDENT: &str = "    ";
const GAP: usize = 3;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const PATTERN: &str = "\x1b[1;31m";
const PATH: &str = "\x1b[1;34m";
const FILTER: &str = "\x1b[33m";
const FLAG: &str = "\x1b[36m";
const NONE: &str = "";

const SHAPES: [(&str, &str); 6] = [
    ("foo", "the pattern to search for"),
    ("src/", "a path that exists: where to search, default here"),
    (".ts  '*.ts'", "only files with this extension"),
    (
        "+10kb  -1mb",
        "only files of at least 10 KB, of at most 1 MB",
    ),
    (
        "+7d  -2h",
        "only files modified over 7 days ago, in the last 2 hours",
    ),
    ("--", "what follows is literal: the pattern, then paths"),
];

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

const EXAMPLES: [(&str, &str); 15] = [
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
    ("2", "an argument or a path was wrong"),
];

const MORE: [(&str, &str); 4] = [
    ("syk -n -h", "the page for one flag"),
    ("syk --size --help", "the same, with the long names"),
    ("syk -h modified", "the same, by name"),
    ("syk foo -x", "show how the arguments are read"),
];

fn note(group: Group) -> Option<&'static str> {
    match group {
        Group::Target => Some(
            "Without these, the pattern is tried on all three and every hit is shown. Two of them with their own pattern narrow each other: -n route -c auth prints the lines matching auth in files named *route*.",
        ),
        Group::Walk => {
            Some("Hidden, ignored and binary files are skipped unless you ask for them.")
        }
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
    if look.banner {
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

fn wrap(out: &mut String, text: &str, start: usize, hang: usize) {
    let mut column = start;
    let mut empty = true;
    for word in text.split_whitespace() {
        let length = word.chars().count();
        if !empty && column + 1 + length > WIDTH {
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
    styled: bool,
}

impl Page {
    fn new(styled: bool) -> Page {
        Page {
            text: String::with_capacity(8192),
            styled,
        }
    }

    fn heading(&mut self, title: &str) {
        self.text.push('\n');
        self.text.push_str(MARGIN);
        put(&mut self.text, BOLD, title, self.styled);
        self.text.push('\n');
    }

    fn paragraph(&mut self, text: &str) {
        self.text.push_str(INDENT);
        wrap(&mut self.text, text, INDENT.len(), INDENT.len());
    }

    fn gap(&mut self) {
        self.text.push('\n');
    }

    fn meaning(&mut self, width: usize, column: usize, text: &str) {
        let start = INDENT.len() + column + GAP;
        self.text
            .push_str(&" ".repeat(start - INDENT.len() - width));
        wrap(&mut self.text, text, start, start);
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

fn overview(look: Look) -> String {
    let styled = look.styled;
    let mut page = Page::new(styled);
    title(&mut page.text, look);

    page.heading("USAGE");
    usage(&mut page.text, styled);

    page.heading("ARGUMENTS");
    page.table(&SHAPES, paint);
    page.gap();
    page.paragraph(SHAPES_NOTE);

    page.heading("PATTERNS");
    page.table(&PATTERNS, paint);

    let every: Vec<&Flag> = FLAGS.iter().collect();
    let column = widest(&every);
    for group in Group::ALL {
        let members: Vec<&Flag> = FLAGS.iter().filter(|flag| flag.group == group).collect();
        page.heading(group.title());
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
    page.text
}

fn topic(page: &mut Page, flag: &Flag) {
    page.text.push('\n');
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

pub fn render(topics: &[Id], look: Look) -> String {
    if topics.is_empty() {
        return overview(look);
    }
    let mut page = Page::new(look.styled);
    page.text.push_str(&banner::header(&about(topics), look));
    for id in topics {
        topic(&mut page, flags::by_id(*id));
    }
    page.heading("MORE HELP");
    page.table(&[("syk -h", "every flag")], paint);
    page.gap();
    page.text
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

pub fn welcome(look: Look) -> String {
    let styled = look.styled;
    let mut out = String::with_capacity(2048);
    title(&mut out, look);
    out.push('\n');
    out.push_str(MARGIN);
    put(&mut out, BOLD, "USAGE", styled);
    out.push('\n');
    usage(&mut out, styled);

    let column = WELCOME
        .iter()
        .flat_map(|(_, rows)| rows.iter())
        .map(|(command, _)| command.len())
        .max()
        .unwrap_or(0);
    for (heading, rows) in WELCOME {
        out.push('\n');
        out.push_str(MARGIN);
        put(&mut out, BOLD, heading, styled);
        out.push('\n');
        for (command, meaning) in rows {
            out.push_str(INDENT);
            paint(&mut out, command, styled);
            out.push_str(&" ".repeat(column - command.len() + GAP));
            out.push_str(meaning);
            if let Some(flag) = only_flag(command) {
                out.push_str("  ");
                put(&mut out, DIM, &format!("or --{}", flag.long), styled);
            }
            out.push('\n');
        }
    }
    out.push('\n');
    out
}

pub fn print_welcome() {
    let stderr = std::io::stderr();
    let look = Look::of(&stderr);
    let _ = stderr.lock().write_all(welcome(look).as_bytes());
}

pub fn print(topics: &[Id]) {
    let stdout = std::io::stdout();
    let look = Look::of(&stdout);
    let _ = stdout.lock().write_all(render(topics, look).as_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIPED: Look = Look {
        banner: false,
        styled: false,
    };
    const SHOWN: Look = Look {
        banner: true,
        styled: false,
    };
    const STYLED: Look = Look {
        banner: true,
        styled: true,
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
