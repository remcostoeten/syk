use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::filter::{self, Parsed};
use crate::flags::{self, Arity, Flag, Id};
use crate::group;
use crate::manage::Action;
use crate::query::{
    Case, Defaults, Expanded, Filters, Format, Kind, Matching, Output, Plan, Query, Targets, Terms,
    Walk,
};

pub enum Outcome {
    Run(Box<Query>),
    Help(Vec<Id>, Vec<String>),
    Version,
    Picks,
    Init(String),
    Setup(Option<String>),
    Indexes(Action, Vec<PathBuf>),
    Usage,
}

#[derive(Clone, PartialEq, Eq)]
enum Slot {
    Off,
    Flagged,
    Bound { pattern: Terms, source: u8 },
}

const BARE: u8 = 0;
const NAME: usize = 0;
const DIR: usize = 1;
const CONTENT: usize = 2;

struct Word {
    text: String,
    raw: OsString,
    forced: bool,
}

enum Shape {
    Root,
    Extension(String),
    Either,
    Pattern,
}

struct Draft {
    slots: [Slot; 3],
    words: Vec<Word>,
    filters: Filters,
    matching: Matching,
    walk: Walk,
    output: Output,
    version: bool,
    picks: bool,
    init: Option<String>,
    setup: Option<Option<String>>,
    indexes: Option<Action>,
    beside: Option<String>,
    no_config: bool,
    inherited: bool,
}

pub fn parse(raw: Vec<OsString>) -> Result<Outcome, String> {
    read(raw, 0)
}

pub fn with_defaults(defaults: Defaults, raw: Vec<OsString>) -> Result<Outcome, String> {
    let inherited = defaults.args.len();
    let mut all: Vec<OsString> = defaults.args.iter().map(OsString::from).collect();
    all.extend(raw);
    let explain = |error: String| {
        format!(
            "{error}\n  The defaults in {} are part of this command. -U skips them.",
            defaults.path.display()
        )
    };
    match read(all, inherited).map_err(explain)? {
        Outcome::Run(mut query) => {
            query.defaults = Some(defaults);
            Ok(Outcome::Run(query))
        }
        other => Ok(other),
    }
}

fn read(raw: Vec<OsString>, inherited: usize) -> Result<Outcome, String> {
    if raw.is_empty() {
        return Ok(Outcome::Usage);
    }
    let args: Vec<String> = raw
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    if let Some((topics, terms)) = help_topics(&args[inherited..]) {
        return Ok(Outcome::Help(topics, terms));
    }

    let mut draft = Draft {
        slots: [Slot::Off, Slot::Off, Slot::Off],
        words: Vec::new(),
        filters: Filters {
            groups: group::presets(),
            ..Filters::default()
        },
        matching: Matching::default(),
        walk: Walk::default(),
        output: Output::default(),
        version: false,
        picks: false,
        init: None,
        setup: None,
        indexes: None,
        beside: None,
        no_config: false,
        inherited: false,
    };
    let mut index = 0;
    let mut forced = false;
    while index < args.len() {
        let arg = &args[index];
        draft.inherited = index < inherited;
        let flag_like = arg.len() > 1 && arg.starts_with('-') && arg != "--";
        if draft.inherited && !flag_like && !filter::is_token(arg) {
            return Err(format!(
                "{arg:?} is not a flag. The config file holds flags and their values only."
            ));
        }
        let word = || Word {
            text: arg.clone(),
            raw: raw[index].clone(),
            forced,
        };
        if forced {
            draft.words.push(word());
            index += 1;
        } else if arg == "--" {
            forced = true;
            index += 1;
        } else if let Some(body) = arg.strip_prefix("--") {
            let (name, inline) = match body.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (body, None),
            };
            let flag = flags::by_long(name).ok_or_else(|| unknown_long(name))?;
            index = draft.apply(flag, inline, &args, index + 1)?;
        } else if filter::is_token(arg) {
            draft.filter(filter::parse_token(arg)?);
            index += 1;
        } else if arg.len() > 1 && arg.starts_with('-') {
            index = draft.cluster(arg, &args, index + 1)?;
        } else {
            draft.words.push(word());
            index += 1;
        }
    }
    if draft.version {
        return Ok(Outcome::Version);
    }
    if draft.picks {
        return Ok(Outcome::Picks);
    }
    if let Some(shell) = draft.init.take() {
        return Ok(Outcome::Init(shell));
    }
    if let Some(shell) = draft.setup.take() {
        return Ok(Outcome::Setup(shell));
    }
    if let Some(action) = draft.indexes.take() {
        if let Some(other) = draft.beside.take() {
            return Err(format!(
                "-A, --indexes takes an action and folders, so {other} cannot go with it."
            ));
        }
        let paths = draft.words.iter().map(|word| PathBuf::from(&word.raw));
        return Ok(Outcome::Indexes(action, paths.collect()));
    }
    draft.finish().map(|query| Outcome::Run(Box::new(query)))
}

struct Seen {
    flags: Vec<(&'static Flag, bool)>,
    words: Vec<String>,
}

fn seen(args: &[String]) -> Seen {
    let mut seen = Seen {
        flags: Vec::new(),
        words: Vec::new(),
    };
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        index += 1;
        if arg == "--" {
            break;
        }
        if filter::is_token(arg) {
            continue;
        }
        let mut open: Option<&'static Flag> = None;
        if let Some(body) = arg.strip_prefix("--") {
            let name = body.split('=').next().unwrap_or(body);
            if let Some(flag) = flags::by_long(name) {
                seen.flags.push((flag, false));
                open = (!body.contains('=')).then_some(flag);
            }
        } else if let Some(body) = arg.strip_prefix('-') {
            if body.chars().count() > 1
                && let Some(flag) = flags::by_long(body)
            {
                seen.flags.push((flag, false));
                open = Some(flag);
            } else {
                for (at, short) in body.char_indices() {
                    let Some(flag) = flags::by_short(short) else {
                        break;
                    };
                    seen.flags.push((flag, false));
                    if !matches!(flag.arity, Arity::Switch) {
                        open = body[at + short.len_utf8()..].is_empty().then_some(flag);
                        break;
                    }
                }
            }
        } else if let Some(flag) = flags::by_long(&arg.to_ascii_lowercase()) {
            seen.flags.push((flag, true));
        } else if !path_like(arg) && extension_token(arg).is_none() {
            seen.words.push(arg.clone());
        }
        let value = args
            .get(index)
            .is_some_and(|next| !(next.len() > 1 && next.starts_with('-')));
        match open.map(|flag| flag.arity) {
            Some(Arity::Required(_)) if value => index += 1,
            Some(Arity::Optional(_)) if value && binds(&args[index]) => index += 1,
            _ => {}
        }
    }
    seen
}

fn help_topics(args: &[String]) -> Option<(Vec<Id>, Vec<String>)> {
    let seen = seen(args);
    let mut asked = false;
    let mut topics: Vec<Id> = Vec::new();
    for (flag, named) in &seen.flags {
        let again = flag.id == Id::Help && asked;
        asked |= flag.id == Id::Help;
        let view = matches!(flag.id, Id::NoPager | Id::NoBanner) && !named;
        if (flag.id != Id::Help || again) && !view && !topics.contains(&flag.id) {
            topics.push(flag.id);
        }
    }
    asked.then_some((topics, seen.words))
}

#[derive(Default)]
pub struct Views {
    pub no_pager: bool,
    pub no_banner: bool,
    pub no_config: bool,
}

pub fn views(args: &[String]) -> Views {
    let mut views = Views::default();
    for (flag, named) in seen(args).flags {
        match flag.id {
            Id::NoPager if !named => views.no_pager = true,
            Id::NoBanner if !named => views.no_banner = true,
            Id::NoConfig if !named => views.no_config = true,
            _ => {}
        }
    }
    views
}

impl Draft {
    fn apply(
        &mut self,
        flag: &Flag,
        inline: Option<String>,
        args: &[String],
        mut index: usize,
    ) -> Result<usize, String> {
        let value = match flag.arity {
            Arity::Switch => {
                if inline.is_some() {
                    return Err(format!("{} does not take a value.", flag.label()));
                }
                None
            }
            Arity::Required(what) => match inline {
                Some(value) => Some(value),
                None => {
                    let value = args.get(index).ok_or_else(|| {
                        format!(
                            "-{}, --{} needs a value: {what}. Run syk -{} -h for examples.",
                            flag.short, flag.long, flag.short
                        )
                    })?;
                    index += 1;
                    Some(value.clone())
                }
            },
            Arity::Optional(_) => match inline {
                Some(value) => Some(value),
                None => match args.get(index) {
                    Some(next) if binds(next) => {
                        index += 1;
                        Some(next.clone())
                    }
                    _ => None,
                },
            },
        };
        self.set(flag, value)?;
        Ok(index)
    }

    fn cluster(&mut self, arg: &str, args: &[String], index: usize) -> Result<usize, String> {
        let body = &arg[1..];
        let (head, inline) = match body.split_once('=') {
            Some((head, value)) => (head, Some(value.to_string())),
            None => (body, None),
        };
        if head.chars().count() > 1
            && let Some(flag) = flags::by_long(head)
        {
            return self.apply(flag, inline, args, index);
        }
        for (at, short) in body.char_indices() {
            let flag = flags::by_short(short).ok_or_else(|| unknown_short(short, arg))?;
            let rest = &body[at + short.len_utf8()..];
            match flag.arity {
                Arity::Switch => self.set(flag, None)?,
                Arity::Required(_) if !rest.is_empty() => {
                    let value = rest.strip_prefix('=').unwrap_or(rest).to_string();
                    return self.apply(flag, Some(value), args, index);
                }
                Arity::Optional(_) if rest.starts_with('=') => {
                    return self.apply(flag, Some(rest[1..].to_string()), args, index);
                }
                Arity::Optional(_) if !rest.is_empty() => self.set(flag, None)?,
                _ => return self.apply(flag, None, args, index),
            }
        }
        Ok(index)
    }

    fn filter(&mut self, parsed: Parsed) {
        if !self.inherited {
            self.beside.get_or_insert_with(|| "a filter".to_string());
        }
        match parsed {
            Parsed::Size(size) => self.filters.sizes.push(size),
            Parsed::Age(age) => self.filters.ages.push(age),
        }
    }

    fn set(&mut self, flag: &Flag, value: Option<String>) -> Result<(), String> {
        let text = value.as_deref().unwrap_or("");
        let neutral = matches!(
            flag.id,
            Id::Indexes | Id::NoPager | Id::NoBanner | Id::NoConfig
        );
        if !self.inherited && !neutral {
            self.beside.get_or_insert_with(|| flag.label());
        }
        if self.inherited
            && matches!(
                flag.id,
                Id::Version
                    | Id::Picks
                    | Id::Init
                    | Id::Setup
                    | Id::Help
                    | Id::NoConfig
                    | Id::Explain
                    | Id::Index
                    | Id::Indexes
            )
        {
            return Err(format!(
                "{} does not belong in the config file.",
                flag.label()
            ));
        }
        let slot = |source: usize| match &value {
            Some(pattern) => Slot::Bound {
                pattern: vec![pattern.clone()],
                source: source as u8 + 1,
            },
            None => Slot::Flagged,
        };
        match flag.id {
            Id::Name => self.slots[NAME] = slot(NAME),
            Id::Dir => self.slots[DIR] = slot(DIR),
            Id::Content => self.slots[CONTENT] = slot(CONTENT),
            Id::Ext => {
                for part in text.split(',') {
                    self.filters.extensions.push(extension(part).ok_or_else(|| {
                        format!(
                            "{part:?} is not an extension. Examples: -e ts, -e ts,tsx, -e d.ts."
                        )
                    })?);
                }
            }
            Id::Exclude => {
                for part in text.split(',').filter(|part| !part.is_empty()) {
                    let part = part.strip_prefix('!').unwrap_or(part);
                    self.filters.excludes.push(part.to_string());
                }
            }
            Id::Ignore => match text.split_once('=') {
                Some((name, patterns)) => group::define(&mut self.filters.groups, name, patterns)?,
                None => group::skip(&mut self.filters.groups, text)?,
            },
            Id::Unignore => group::search(&mut self.filters.groups, text)?,
            Id::Size => self.filters.sizes.push(filter::parse_size(text)?),
            Id::Modified => self.filters.ages.push(filter::parse_age(text)?),
            Id::Type => self.filters.kind = Some(kind(text)?),
            Id::Depth => self.walk.depth = Some(number(flag, text)?),
            Id::IgnoreCase => self.matching.case = Case::Insensitive,
            Id::CaseSensitive => self.matching.case = Case::Sensitive,
            Id::Literal => self.matching.literal = true,
            Id::Word => self.matching.word = true,
            Id::Exact => self.output.exact = true,
            Id::Hidden => self.walk.hidden = true,
            Id::NoIgnore => self.walk.no_ignore = true,
            Id::All => self.walk.all = true,
            Id::Follow => self.walk.follow = true,
            Id::List => self.output.list = true,
            Id::Max => self.output.max = Some(number(flag, text)?),
            Id::Context => self.output.context = number(flag, text)?,
            Id::Sort => self.walk.sort = true,
            Id::Threads => self.walk.threads = number(flag, text)?,
            Id::Index => self.walk.index = true,
            Id::NoIndex => self.walk.no_index = true,
            Id::Indexes => self.indexes = Some(Action::of(text)?),
            Id::Plain => self.output.format = Format::Plain,
            Id::Pretty => self.output.format = Format::Pretty,
            Id::Explain => self.output.explain = true,
            Id::Rank => self.output.rank = Some(true),
            Id::NoRank => self.output.rank = Some(false),
            Id::Best => self.output.best = true,
            Id::Version => self.version = true,
            Id::Why => self.output.why = true,
            Id::Picks => self.picks = true,
            Id::Count => self.output.count = true,
            Id::Stats => self.output.stats = true,
            Id::Json => self.output.json = true,
            Id::Null => self.output.null = true,
            Id::NoLinks => self.output.no_links = true,
            Id::Open => self.output.open = true,
            Id::Browse => self.output.browse = true,
            Id::NoPager | Id::NoBanner => {}
            Id::Exec => {
                if text.trim().is_empty() {
                    return Err(
                        "-f, --exec needs a command, for example: syk .log -f 'gzip {}'"
                            .to_string(),
                    );
                }
                self.output.exec = Some(text.to_string());
            }
            Id::NoConfig => self.no_config = true,
            Id::Init => self.init = Some(text.to_string()),
            Id::Setup => self.setup = Some(value.clone()),
            Id::Help => {}
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Query, String> {
        let mut patterns: Vec<String> = Vec::new();
        let mut either: Vec<Word> = Vec::new();
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut first_forced = true;
        for word in std::mem::take(&mut self.words) {
            if word.forced {
                if first_forced {
                    patterns.push(word.text);
                } else if Path::new(&word.raw).exists() {
                    roots.push(PathBuf::from(word.raw));
                } else {
                    return Err(missing(&word.text));
                }
                first_forced = false;
                continue;
            }
            match shape(&word)? {
                Shape::Root => roots.push(PathBuf::from(word.raw)),
                Shape::Extension(extension) => self.filters.extensions.push(extension),
                Shape::Either => either.push(word),
                Shape::Pattern => patterns.push(word.text),
            }
        }
        if self.output.open && self.output.exec.is_some() {
            return Err(
                "-O opens the best result and -f runs a command on every result. Use one of them."
                    .to_string(),
            );
        }
        if self.output.json && self.output.null {
            return Err("-J and -0 are two different output formats. Use one of them.".to_string());
        }
        let bound = self
            .slots
            .iter()
            .any(|slot| matches!(slot, Slot::Bound { .. }));
        let either: Vec<PathBuf> = either
            .into_iter()
            .map(|word| PathBuf::from(word.raw))
            .collect();
        let wildcard = patterns.is_empty() && either.len() > 1 && expansion(&either).is_some();
        let mut either = either.into_iter();
        let bare = if !patterns.is_empty() {
            Some(patterns)
        } else if !bound && !wildcard {
            either
                .next()
                .map(|path| vec![path.to_string_lossy().into_owned()])
        } else {
            None
        };
        roots.extend(either);
        let expanded = expansion(&roots);
        if roots.is_empty() {
            roots.push(PathBuf::from("."));
        }
        group::check(&self.filters.groups)?;
        Ok(Query {
            plan: plan(self.slots, bare)?,
            roots,
            filters: self.filters,
            matching: self.matching,
            walk: self.walk,
            output: self.output,
            expanded,
            no_config: self.no_config,
            defaults: None,
        })
    }
}

fn expansion(roots: &[PathBuf]) -> Option<Expanded> {
    let mut shared: Option<&str> = None;
    for root in roots {
        let name = root.to_str()?;
        let (stem, extension) = name.rsplit_once('.')?;
        if name.contains('/') || stem.is_empty() || extension.is_empty() {
            return None;
        }
        if *shared.get_or_insert(extension) != extension {
            return None;
        }
    }
    let extension = format!(".{}", shared?);
    let present = std::fs::read_dir(".")
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && name.len() > extension.len() && name.ends_with(&extension)
        })
        .count();
    (present == roots.len()).then_some(Expanded {
        extension,
        files: roots.len(),
    })
}

fn plan(mut slots: [Slot; 3], bare: Option<Terms>) -> Result<Plan, String> {
    let flagged = slots.iter().filter(|slot| **slot == Slot::Flagged).count();
    let bound: Vec<Slot> = slots
        .iter()
        .filter(|slot| matches!(slot, Slot::Bound { .. }))
        .cloned()
        .collect();
    let fill = |slots: &mut [Slot; 3], with: &Slot| {
        for slot in slots.iter_mut().filter(|slot| **slot == Slot::Flagged) {
            *slot = with.clone();
        }
    };
    match bare {
        Some(pattern) => {
            let bare = Slot::Bound {
                pattern,
                source: BARE,
            };
            if flagged > 0 {
                fill(&mut slots, &bare);
            } else if bound.is_empty() {
                slots = [bare.clone(), bare.clone(), bare];
            } else {
                let free = [CONTENT, NAME, DIR]
                    .into_iter()
                    .find(|index| slots[*index] == Slot::Off)
                    .ok_or("-n, -d and -c each have a pattern already, so the extra word has no place. Remove it, or end it with a slash if it is a path.")?;
                slots[free] = bare;
            }
        }
        None if flagged > 0 && bound.len() == 1 => fill(&mut slots, &bound[0]),
        None if flagged > 0 && bound.len() > 1 => {
            return Err("one of -n, -d, -c has no pattern while the others each have their own. Give it a pattern, or remove it.".to_string());
        }
        None if flagged > 0 => {
            if slots[CONTENT] == Slot::Flagged {
                return Err("-c needs a pattern, for example: syk -c foo. A pattern that looks like a flag, a filter or a path can be attached with an equals sign: -c=.env".to_string());
            }
            return Ok(Plan::Union {
                pattern: None,
                targets: Targets {
                    dirs: true,
                    files: slots[NAME] == Slot::Flagged,
                    content: false,
                },
            });
        }
        None if bound.is_empty() => {
            return Ok(Plan::Union {
                pattern: None,
                targets: Targets {
                    dirs: true,
                    files: true,
                    content: false,
                },
            });
        }
        None => {}
    }

    let mut sources: Vec<u8> = Vec::new();
    let mut patterns: [Option<Terms>; 3] = [None, None, None];
    for (index, slot) in slots.into_iter().enumerate() {
        if let Slot::Bound { pattern, source } = slot {
            if !sources.contains(&source) {
                sources.push(source);
            }
            patterns[index] = Some(pattern);
        }
    }
    let [name, dir, content] = patterns;
    if sources.len() > 1 {
        return Ok(Plan::Narrow { dir, name, content });
    }
    Ok(Plan::Union {
        targets: Targets {
            dirs: name.is_some() || dir.is_some(),
            files: name.is_some(),
            content: content.is_some(),
        },
        pattern: name.or(dir).or(content),
    })
}

fn binds(next: &str) -> bool {
    let flag_like = next.len() > 1 && next.starts_with('-');
    !flag_like && !filter::is_token(next) && extension_token(next).is_none() && !path_like(next)
}

fn path_like(text: &str) -> bool {
    text == "."
        || text == ".."
        || text.ends_with('/')
        || text.starts_with("./")
        || text.starts_with("../")
        || text.starts_with('/')
        || text.starts_with("~/")
}

fn shape(word: &Word) -> Result<Shape, String> {
    let exists = Path::new(&word.raw).exists();
    if path_like(&word.text) {
        return if exists {
            Ok(Shape::Root)
        } else if word.text.starts_with('/') && !word.text.ends_with('/') {
            Ok(Shape::Pattern)
        } else {
            Err(missing(&word.text))
        };
    }
    if let Some(extension) = extension_token(&word.text) {
        return Ok(if exists {
            Shape::Root
        } else {
            Shape::Extension(extension)
        });
    }
    Ok(if exists {
        Shape::Either
    } else {
        Shape::Pattern
    })
}

fn extension_token(text: &str) -> Option<String> {
    let body = text.strip_prefix('*').unwrap_or(text);
    if !body.starts_with('.') {
        return None;
    }
    extension(body)
}

fn extension(text: &str) -> Option<String> {
    let body = text.strip_prefix('*').unwrap_or(text);
    let body = body.strip_prefix('.').unwrap_or(body);
    let valid = !body.is_empty()
        && body.split('.').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '-'))
        });
    valid.then(|| format!(".{}", body.to_lowercase()))
}

fn kind(text: &str) -> Result<Kind, String> {
    match text.to_ascii_lowercase().as_str() {
        "f" | "file" | "files" => Ok(Kind::File),
        "d" | "dir" | "dirs" | "directory" | "directories" | "folder" | "folders" => Ok(Kind::Dir),
        "l" | "link" | "links" | "symlink" | "symlinks" => Ok(Kind::Link),
        _ => Err(format!(
            "{text:?} is not a kind of entry. Use file, dir or link."
        )),
    }
}

fn number(flag: &Flag, text: &str) -> Result<usize, String> {
    text.parse().map_err(|_| {
        format!(
            "-{}, --{} needs a whole number, got {text:?}.",
            flag.short, flag.long
        )
    })
}

fn unknown_long(name: &str) -> String {
    match flags::closest(name) {
        Some(flag) => format!(
            "unknown flag --{name}. Did you mean --{}? Run syk -h for every flag.",
            flag.long
        ),
        None => format!("unknown flag --{name}. Run syk -h for every flag."),
    }
}

fn unknown_short(short: char, arg: &str) -> String {
    let swapped = match short.is_ascii_lowercase() {
        true => short.to_ascii_uppercase(),
        false => short.to_ascii_lowercase(),
    };
    let like = match arg.chars().count() {
        2 => flags::by_short(swapped),
        _ => flags::closest(&arg[1..]),
    };
    let hint = match like {
        Some(flag) => format!(" Did you mean -{}, --{}?", flag.short, flag.long),
        None => String::new(),
    };
    if arg.chars().count() == 2 {
        format!(
            "unknown flag {arg}.{hint} Run syk -h for every flag. To search for {arg} itself, write: syk -- {arg}"
        )
    } else {
        format!(
            "unknown flag -{short} in {arg}.{hint} Run syk -h for every flag. To search for {arg} itself, write: syk -- {arg}"
        )
    }
}

fn missing(text: &str) -> String {
    format!(
        "no such file or folder: {text}. To search for it as text instead, write: syk -- {text}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(args: &[&str]) -> Query {
        match parse(args.iter().map(OsString::from).collect()) {
            Ok(Outcome::Run(query)) => *query,
            Ok(_) => panic!("expected a search for {args:?}"),
            Err(error) => panic!("{args:?} failed: {error}"),
        }
    }

    fn error(args: &[&str]) -> String {
        match parse(args.iter().map(OsString::from).collect()) {
            Err(error) => error,
            Ok(_) => panic!("expected an error for {args:?}"),
        }
    }

    fn first(terms: &Option<Terms>) -> Option<&str> {
        terms.as_ref().map(|terms| terms[0].as_str())
    }

    fn union(query: &Query) -> (Option<&str>, bool, bool, bool) {
        match &query.plan {
            Plan::Union { pattern, targets } => {
                (first(pattern), targets.dirs, targets.files, targets.content)
            }
            Plan::Narrow { .. } => panic!("expected a union plan"),
        }
    }

    fn narrow(query: &Query) -> (Option<&str>, Option<&str>, Option<&str>) {
        match &query.plan {
            Plan::Narrow { dir, name, content } => (first(dir), first(name), first(content)),
            Plan::Union { .. } => panic!("expected a narrow plan"),
        }
    }

    #[test]
    fn bare_pattern_searches_everything() {
        assert_eq!(union(&query(&["zzfoo"])), (Some("zzfoo"), true, true, true));
    }

    #[test]
    fn mode_flags_restrict_the_pattern() {
        assert_eq!(
            union(&query(&["-n", "zzfoo"])),
            (Some("zzfoo"), true, true, false)
        );
        assert_eq!(
            union(&query(&["zzfoo", "-n"])),
            (Some("zzfoo"), true, true, false)
        );
        assert_eq!(
            union(&query(&["-d", "zzfoo"])),
            (Some("zzfoo"), true, false, false)
        );
        assert_eq!(
            union(&query(&["-c", "zzfoo"])),
            (Some("zzfoo"), false, false, true)
        );
        assert_eq!(
            union(&query(&["--content=zzfoo"])),
            (Some("zzfoo"), false, false, true)
        );
        for long in ["--dir", "--directory", "--directories", "--folder"] {
            assert_eq!(
                union(&query(&[long, "zzfoo"])),
                (Some("zzfoo"), true, false, false)
            );
        }
        assert_eq!(
            union(&query(&["-name", "zzfoo"])),
            (Some("zzfoo"), true, true, false)
        );
    }

    #[test]
    fn flags_without_a_pattern_list() {
        assert_eq!(union(&query(&["-n"])), (None, true, true, false));
        assert_eq!(union(&query(&["-d"])), (None, true, false, false));
        assert!(error(&["-c"]).contains("needs a pattern"));
    }

    #[test]
    fn separate_patterns_narrow_each_other() {
        assert_eq!(
            narrow(&query(&["-n", "zzroute", "-c", "zzauth"])),
            (None, Some("zzroute"), Some("zzauth"))
        );
        assert_eq!(
            narrow(&query(&["-n", "zzroute", "zzauth"])),
            (None, Some("zzroute"), Some("zzauth"))
        );
        assert_eq!(
            narrow(&query(&["-d", "zzapi", "-c", "zzauth"])),
            (Some("zzapi"), None, Some("zzauth"))
        );
    }

    #[test]
    fn tokens_are_read_by_shape() {
        let query = query(&["zzfoo", ".ts", "*.TSX", "+10kb", "-2d", "src/"]);
        assert_eq!(query.filters.extensions, [".ts", ".tsx"]);
        assert_eq!(query.filters.sizes[0].bytes, 10 * 1024);
        assert!(query.filters.sizes[0].at_least);
        assert_eq!(query.filters.ages[0].seconds, 2 * 24 * 60 * 60);
        assert!(!query.filters.ages[0].older);
        assert_eq!(query.roots, [PathBuf::from("src/")]);
    }

    #[test]
    fn an_existing_word_is_the_pattern_only_when_alone() {
        let alone = query(&["src"]);
        assert_eq!(union(&alone).0, Some("src"));
        assert_eq!(alone.roots, [PathBuf::from(".")]);

        let beside = query(&["zzfoo", "src"]);
        assert_eq!(union(&beside).0, Some("zzfoo"));
        assert_eq!(beside.roots, [PathBuf::from("src")]);

        let flipped = query(&["src", "zzfoo"]);
        assert_eq!(union(&flipped).0, Some("zzfoo"));
        assert_eq!(flipped.roots, [PathBuf::from("src")]);
    }

    #[test]
    fn short_flags_cluster_and_take_values() {
        let query = query(&["-il", "-m5", "-D", "3", "-e", "rs,toml", "zzfoo"]);
        assert!(query.output.list);
        assert_eq!(query.output.max, Some(5));
        assert_eq!(query.walk.depth, Some(3));
        assert_eq!(query.filters.extensions, [".rs", ".toml"]);
        assert!(query.matching.case == Case::Insensitive);
    }

    #[test]
    fn double_dash_ends_inference() {
        let query = query(&["--", "-n", "src"]);
        assert_eq!(union(&query), (Some("-n"), true, true, true));
        assert_eq!(query.roots, [PathBuf::from("src")]);
    }

    #[test]
    fn mistakes_get_a_pointer() {
        assert!(error(&["--hiden"]).contains("--hidden"));
        assert!(error(&["--directorys"]).contains("--dir?"));
        assert!(error(&["zzfoo", "zznope/"]).contains("no such file or folder"));
        assert!(error(&["+10"]).contains("no unit"));
        assert!(error(&["-m"]).contains("needs a value"));
    }

    #[test]
    fn a_wildcard_the_shell_expanded_is_noticed() {
        let mut manifests: Vec<String> = std::fs::read_dir(".")
            .expect("the project folder")
            .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
            .filter(|name| name.ends_with(".toml"))
            .collect();
        manifests.sort();
        let mut args = vec!["zzfoo"];
        args.extend(manifests.iter().map(String::as_str));
        let expanded = query(&args).expanded;
        assert_eq!(
            expanded.map(|found| (found.extension, found.files)),
            Some((".toml".to_string(), manifests.len()))
        );
        assert!(query(&["zzfoo", "src"]).expanded.is_none());
        assert!(query(&["zzfoo", "src/main.rs"]).expanded.is_none());
        assert!(query(&["zzfoo", ".toml"]).expanded.is_none());
    }

    #[test]
    fn excludes_are_collected() {
        let found = query(&["zzfoo", "-E", "dist,build", "--exclude=!*.min.js", "-X"]);
        assert_eq!(found.filters.excludes, ["dist", "build", "*.min.js"]);
        assert!(found.output.exact);
        assert!(error(&["zzfoo", "-E"]).contains("needs a value"));
    }

    #[test]
    fn ranking_flags_are_read() {
        assert_eq!(query(&["-n", "zzfoo"]).output.rank, None);
        assert_eq!(query(&["-n", "zzfoo", "-r"]).output.rank, Some(true));
        assert_eq!(
            query(&["-n", "zzfoo", "--no-rank"]).output.rank,
            Some(false)
        );
        let best = query(&["-d", "zzfoo", "-1"]);
        assert!(best.output.best);
        assert!(best.filters.sizes.is_empty() && best.filters.ages.is_empty());
        assert!(query(&["zzfoo", "--best"]).output.best);
        assert!(query(&["zzfoo", "-W"]).output.why);
        assert!(matches!(
            parse(vec![OsString::from("--picks")]),
            Ok(Outcome::Picks)
        ));
    }

    #[test]
    fn several_words_are_one_pattern_with_several_terms() {
        let both = query(&["zzfoo", "zzbar", "src"]);
        match &both.plan {
            Plan::Union { pattern, .. } => {
                assert_eq!(
                    pattern.as_deref(),
                    Some(&["zzfoo".to_string(), "zzbar".to_string()][..])
                );
            }
            Plan::Narrow { .. } => panic!("expected a union plan"),
        }
        assert_eq!(both.roots, [PathBuf::from("src")]);
        match &query(&["-n", "zzroute", "zzfoo", "zzbar"]).plan {
            Plan::Narrow { content, .. } => assert_eq!(content.as_ref().map(Vec::len), Some(2)),
            Plan::Union { .. } => panic!("expected a narrow plan"),
        }
    }

    #[test]
    fn output_and_action_flags_are_read() {
        let found = query(&["zzfoo", "-N", "-T", "-J", "-K"]);
        assert!(found.output.count && found.output.stats && found.output.json);
        assert!(found.output.no_links);
        let null = query(&["zzfoo", "-0"]);
        assert!(null.output.null && null.filters.sizes.is_empty());
        assert!(query(&["zzfoo", "-l0"]).output.null);
        assert!(query(&["zzfoo", "--open"]).output.open);
        assert_eq!(
            query(&["zzfoo", "-f", "wc -l"]).output.exec.as_deref(),
            Some("wc -l")
        );
        assert!(error(&["zzfoo", "-O", "-f", "wc"]).contains("Use one of them"));
        assert!(error(&["zzfoo", "-J", "-0"]).contains("Use one of them"));
        assert!(query(&["zzfoo", "-U"]).no_config);
        assert!(matches!(
            parse(vec![OsString::from("--init"), OsString::from("fish")]),
            Ok(Outcome::Init(shell)) if shell == "fish"
        ));
        assert!(matches!(
            parse(vec![OsString::from("--setup")]),
            Ok(Outcome::Setup(None))
        ));
        assert!(matches!(
            parse(vec![OsString::from("-Z"), OsString::from("zsh:j")]),
            Ok(Outcome::Setup(Some(shell))) if shell == "zsh:j"
        ));
    }

    #[test]
    fn defaults_from_the_config_file_come_first() {
        let defaults = |args: &[&str]| Defaults {
            path: PathBuf::from("/config"),
            args: args.iter().map(|arg| arg.to_string()).collect(),
        };
        let merged = |config: &[&str], args: &[&str]| {
            with_defaults(defaults(config), args.iter().map(OsString::from).collect())
        };
        match merged(&["-E", "dist", "--stats", "-p"], &["zzfoo", "-P"]) {
            Ok(Outcome::Run(query)) => {
                assert_eq!(query.filters.excludes, ["dist"]);
                assert!(query.output.stats);
                assert!(query.output.format == Format::Pretty);
                assert!(query.defaults.is_some());
            }
            _ => panic!("expected a search"),
        }
        for config in [&["zzbar"][..], &["-V"], &["--", "x"], &[".ts"]] {
            let Err(error) = merged(config, &["zzfoo"]) else {
                panic!("{config:?} should be refused");
            };
            assert!(error.contains("/config"), "{error}");
        }
    }

    #[test]
    fn help_collects_topics() {
        let asked = |args: &[&str]| match parse(args.iter().map(OsString::from).collect()) {
            Ok(Outcome::Help(topics, terms)) => (topics, terms),
            _ => panic!("expected help for {args:?}"),
        };
        let topics = |args: &[&str]| asked(args).0;
        assert!(topics(&["-h"]).is_empty());
        assert_eq!(topics(&["-n", "-h"]), [Id::Name]);
        assert_eq!(topics(&["--size", "--help"]), [Id::Size]);
        assert_eq!(topics(&["-h", "modified"]), [Id::Modified]);
        assert_eq!(topics(&["-lh"]), [Id::List]);
        assert_eq!(topics(&["-1", "-h"]), [Id::Best]);
        assert_eq!(topics(&["-h", "-h"]), [Id::Help]);
        assert_eq!(topics(&["-h", "help"]), [Id::Help]);
        assert_eq!(topics(&["-n", "--help", "-h"]), [Id::Name, Id::Help]);
        assert!(topics(&["-h", "-G", "-B"]).is_empty());
        assert_eq!(topics(&["-h", "no-pager"]), [Id::NoPager]);
        assert_eq!(topics(&["-c", "json", "-h"]), [Id::Content]);
        assert_eq!(topics(&["-e", "ts", "-h"]), [Id::Ext]);
        let (topics, terms) = asked(&["-h", "pipe", "src/", ".ts"]);
        assert!(topics.is_empty());
        assert_eq!(terms, ["pipe"]);
    }

    #[test]
    fn view_flags_are_found_among_the_other_arguments() {
        let words =
            |args: &[&str]| -> Vec<String> { args.iter().map(|arg| arg.to_string()).collect() };
        let views = views(&words(&["foo", "-xG", "--no-banner"]));
        assert!(views.no_pager && views.no_banner && !views.no_config);
        let views = super::views(&words(&["-f", "echo -G", "-U", "--", "-B"]));
        assert!(!views.no_pager && !views.no_banner && views.no_config);
        assert!(!super::views(&words(&["-h", "no-pager"])).no_pager);
        assert_eq!(query(&["-n", "zzfoo", "-1h"]).filters.ages[0].seconds, 3600);
    }
}
