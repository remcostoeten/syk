use std::collections::{HashSet, VecDeque};
use std::ffi::{OsStr, OsString};
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use grep_matcher::Matcher;
use grep_searcher::sinks::Bytes;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use ignore::overrides::{Override, OverrideBuilder};
use ignore::{DirEntry, WalkBuilder, WalkState};

use crate::act;
use crate::group::{self, Group, Skip, Skips, ends_with};
use crate::index::{self, Fresh, Index, Opened, Scan, Visit};
use crate::item::{Form, Item};
use crate::load::{self, Loaded};
use crate::output::{Budget, Entry, Layout, Lines, Style, write_count, write_entry};
use crate::pattern::Pattern;
use crate::pick;
use crate::query::{Filters, Format, Kind, Matching, Plan, Query, Targets, Terms};
use crate::rank::{self, Block, Habits, Score};
use crate::spell::{self, Gather, Seen};
use crate::stop;

const CHUNK: usize = 64 * 1024;
const WAIT: Duration = Duration::from_millis(10);
const AHEAD: usize = 32;
const FEW: usize = 128;
const PATIENCE: Duration = Duration::from_millis(2);
const WORKERS: usize = 12;
const PIECE: u32 = 2048;
const BIG: u64 = 20_000;

#[allow(clippy::large_enum_variant)]
pub enum Compiled {
    Union {
        pattern: Option<Pattern>,
        targets: Targets,
    },
    Narrow {
        dir: Option<Pattern>,
        name: Option<Pattern>,
        content: Option<Pattern>,
    },
}

impl Compiled {
    fn content(&self) -> Option<&Pattern> {
        match self {
            Compiled::Union { pattern, targets } => pattern.as_ref().filter(|_| targets.content),
            Compiled::Narrow { content, .. } => content.as_ref(),
        }
    }
}

pub fn compile(query: &Query) -> Result<Compiled, String> {
    let build = |terms: &Option<Terms>| -> Result<Option<Pattern>, String> {
        terms
            .as_deref()
            .map(|terms| Pattern::compile(terms, &query.matching))
            .transpose()
    };
    Ok(match &query.plan {
        Plan::Union { pattern, targets } => Compiled::Union {
            pattern: build(pattern)?,
            targets: *targets,
        },
        Plan::Narrow { dir, name, content } => Compiled::Narrow {
            dir: build(dir)?,
            name: build(name)?,
            content: build(content)?,
        },
    })
}

struct Shared {
    plan: Compiled,
    filters: Filters,
    unread: Skips,
    follow: bool,
    folders: bool,
    stat: bool,
    cutoffs: Vec<(bool, SystemTime)>,
    layout: Layout,
    color: bool,
    json: bool,
    end: u8,
    base: Option<Vec<u8>>,
    live: bool,
    context: usize,
    budget: Budget,
    found: AtomicBool,
    failed: AtomicBool,
    quiet: bool,
    gather: bool,
    fold: bool,
    seen: Mutex<Seen>,
    keep: bool,
    blocks: Mutex<Vec<Block>>,
    tally: Mutex<Tally>,
}

#[derive(Clone, Copy, Default)]
struct Tally {
    entries: u64,
    read: u64,
    lines: u64,
    files: u64,
    names: u64,
    withheld: Withheld,
}

#[derive(Clone, Copy, Default)]
struct Withheld {
    files: u64,
    lines: u64,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.entries += other.entries;
        self.read += other.read;
        self.lines += other.lines;
        self.files += other.files;
        self.names += other.names;
        self.withheld.files += other.withheld.files;
        self.withheld.lines += other.withheld.lines;
    }
}

impl Shared {
    fn style(&self) -> Style<'_> {
        Style {
            color: self.color,
            json: self.json,
            end: self.end,
            base: self.base.as_deref(),
        }
    }

    fn fail(&self, message: impl std::fmt::Display) {
        self.failed.store(true, Ordering::Relaxed);
        if !self.quiet {
            eprintln!("syk: {message}");
        }
    }
}

#[derive(Clone, Copy)]
struct View {
    layout: Layout,
    color: bool,
    live: bool,
    json: bool,
    end: u8,
    links: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pass {
    First,
    Gather { fold: bool },
    Again,
}

type Found = Result<DirEntry, ignore::Error>;

#[derive(Clone, Copy)]
struct Job {
    hit: Option<(usize, usize)>,
    named: bool,
    plain: bool,
    muted: bool,
}

struct Waiting {
    path: PathBuf,
    depth: usize,
    form: Form,
    job: Job,
}

struct Outcome {
    found: bool,
    failed: bool,
    seen: Seen,
    blocks: Vec<Block>,
    tally: Tally,
    advice: Advice,
    reread: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Advice {
    Leave,
    Large,
    Build,
}

struct Stored {
    index: Index,
    dirty: Vec<u32>,
    fresh: Option<Fresh>,
}

struct Worker<'a> {
    shared: &'a Shared,
    searcher: Searcher,
    loaded: Vec<u8>,
    ahead: usize,
    waiting: VecDeque<Waiting>,
    out: Vec<u8>,
    flushed: Instant,
    scope: (OsString, bool),
    seen: Seen,
    scratch: Seen,
    blocks: Vec<Block>,
    quality: f32,
    line: Option<u64>,
    tally: Tally,
}

impl<'a> Worker<'a> {
    fn new(shared: &'a Shared, ahead: usize) -> Worker<'a> {
        let searcher = SearcherBuilder::new()
            .binary_detection(BinaryDetection::quit(0))
            .line_number(true)
            .before_context(shared.context)
            .after_context(shared.context)
            .build();
        Worker {
            shared,
            searcher,
            loaded: Vec::new(),
            ahead,
            waiting: VecDeque::new(),
            out: Vec::with_capacity(CHUNK),
            flushed: Instant::now(),
            scope: (OsString::new(), false),
            seen: Seen::new(),
            scratch: Seen::new(),
            blocks: Vec::new(),
            quality: 0.0,
            line: None,
            tally: Tally::default(),
        }
    }

    fn visit(&mut self, result: Found) -> WalkState {
        if self.shared.budget.stopped() {
            return WalkState::Quit;
        }
        let entry = match result {
            Ok(entry) => entry,
            Err(error) => {
                self.shared.fail(error);
                return WalkState::Continue;
            }
        };
        let Some(kind) = entry.file_type() else {
            return WalkState::Continue;
        };
        self.see(&Item {
            path: entry.path(),
            name: entry.file_name(),
            depth: entry.depth(),
            form: Form::of(kind),
            follow: self.shared.follow,
        })
    }

    fn see(&mut self, entry: &Item<'_>) -> WalkState {
        if self.shared.budget.stopped() {
            return WalkState::Quit;
        }
        let kind = entry.form;
        self.tally.entries += 1;
        if kind.is_dir() {
            self.quality = 0.0;
            self.line = None;
            if entry.depth() > 0 {
                self.folder(entry);
            }
            self.close(entry);
            return WalkState::Continue;
        }
        let Some(job) = self.wanted(entry, kind) else {
            self.close(entry);
            return WalkState::Continue;
        };
        if !self.reads(job) {
            self.handle(entry, job, None);
            return WalkState::Continue;
        }
        let patient = self.ahead == 0;
        match self.load(entry.path(), patient) {
            Err(error) if !patient && error.kind() == io::ErrorKind::WouldBlock => {
                self.waiting.push_back(Waiting {
                    path: entry.path().to_path_buf(),
                    depth: entry.depth(),
                    form: entry.form,
                    job,
                });
                if self.waiting.len() > self.ahead {
                    self.next();
                }
            }
            loaded => self.handle(entry, job, Some(loaded)),
        }
        WalkState::Continue
    }

    fn take(&mut self, visit: Visit<'_>) -> bool {
        match visit {
            Visit::Entry(entry) => !matches!(self.see(&entry), WalkState::Quit),
            Visit::Failure(text) => {
                self.shared.fail(text);
                true
            }
        }
    }

    fn load(&mut self, path: &Path, patient: bool) -> io::Result<Loaded> {
        load::fill(File::open(path)?, &mut self.loaded, patient)
    }

    fn reads(&self, job: Job) -> bool {
        let shared = self.shared;
        job.plain
            && !shared.gather
            && shared.plan.content().is_some()
            && !(job.named && shared.layout == Layout::List)
    }

    fn next(&mut self) {
        let Some(waiting) = self.waiting.pop_front() else {
            return;
        };
        let loaded = self.load(&waiting.path, true);
        let entry = Item {
            path: &waiting.path,
            name: waiting.path.file_name().unwrap_or(waiting.path.as_os_str()),
            depth: waiting.depth,
            form: waiting.form,
            follow: self.shared.follow,
        };
        self.handle(&entry, waiting.job, Some(loaded));
    }

    fn handle(&mut self, entry: &Item<'_>, job: Job, loaded: Option<io::Result<Loaded>>) {
        self.quality = 0.0;
        self.line = None;
        self.file(entry, job, loaded);
        self.close(entry);
    }

    fn close(&mut self, entry: &Item<'_>) {
        if self.shared.keep {
            self.keep(entry);
        } else {
            self.settle();
        }
    }

    fn keep(&mut self, entry: &Item<'_>) {
        if self.out.is_empty() {
            return;
        }
        self.blocks.push(Block {
            path: display(entry.path()).to_vec(),
            quality: self.quality,
            depth: entry.depth(),
            modified: entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok()),
            line: self.line,
            text: std::mem::take(&mut self.out),
        });
    }

    fn folder(&mut self, entry: &Item<'_>) {
        let shared = self.shared;
        if !shared.folders {
            return;
        }
        let name = entry.file_name().as_bytes();
        let hit = match &shared.plan {
            Compiled::Union { pattern, targets } => {
                if !targets.dirs {
                    return;
                }
                match pattern {
                    None => None,
                    Some(pattern) => match pattern.find(name) {
                        Some(found) => Some(found),
                        None => return,
                    },
                }
            }
            Compiled::Narrow {
                dir,
                name: wanted,
                content,
            } => {
                let (Some(wanted), None) = (wanted, content) else {
                    return;
                };
                if let Some(dir) = dir
                    && !self.in_scope(entry, dir)
                {
                    return;
                }
                match wanted.find(name) {
                    Some(found) => Some(found),
                    None => return,
                }
            }
        };
        let path = display(entry.path());
        self.quality = rank::quality(name, hit);
        self.path_line(
            &Entry {
                path,
                name_at: path.len().saturating_sub(name.len()),
                hit,
                folder: true,
            },
            true,
        );
    }

    fn wanted(&mut self, entry: &Item<'_>, kind: Form) -> Option<Job> {
        let shared = self.shared;
        match shared.filters.kind {
            Some(Kind::Dir) => return None,
            Some(Kind::File) if !kind.is_file() => return None,
            Some(Kind::Link) if !kind.is_symlink() => return None,
            _ => {}
        }
        let name = entry.file_name().as_bytes();
        let extensions = &shared.filters.extensions;
        if !extensions.is_empty()
            && !extensions
                .iter()
                .any(|extension| ends_with(name, extension.as_bytes()))
        {
            return None;
        }
        if shared.stat && !self.passes_stat(entry) {
            return None;
        }
        let (hit, alone) = match &shared.plan {
            Compiled::Union { pattern, targets } => {
                let hit = match pattern {
                    _ if !targets.files => None,
                    None => Some(None),
                    Some(pattern) => pattern.find(name).map(Some),
                };
                (hit, true)
            }
            Compiled::Narrow {
                dir,
                name: wanted,
                content,
            } => {
                if let Some(dir) = dir
                    && !self.in_scope(entry, dir)
                {
                    return None;
                }
                let hit = match wanted {
                    None => None,
                    Some(wanted) => Some(Some(wanted.find(name)?)),
                };
                (hit, content.is_none())
            }
        };
        let muted = kind.is_file()
            && shared.plan.content().is_some()
            && shared.unread.by_name()
            && self.unread(entry);
        let job = Job {
            hit: hit.flatten(),
            named: alone && hit.is_some(),
            plain: kind.is_file() && !muted,
            muted,
        };
        (job.named || job.muted || (job.plain && shared.plan.content().is_some())).then_some(job)
    }

    fn unread(&self, entry: &Item<'_>) -> bool {
        entry
            .path()
            .as_os_str()
            .as_bytes()
            .rsplit(|byte| *byte == b'/')
            .take(entry.depth())
            .any(|part| self.shared.unread.matches(part))
    }

    fn weigh(&mut self, entry: &Item<'_>) {
        let Some(pattern) = self.shared.plan.content() else {
            return;
        };
        let mut lines = 0;
        let count = Bytes(|_, _| {
            lines += 1;
            Ok(true)
        });
        let searched = if pattern.single() {
            self.searcher
                .search_path(pattern.lines(), entry.path(), count)
        } else {
            self.searcher.search_path(pattern, entry.path(), count)
        };
        if searched.is_ok() && lines > 0 {
            self.tally.withheld.files += 1;
            self.tally.withheld.lines += lines;
        }
    }

    fn file(&mut self, entry: &Item<'_>, job: Job, loaded: Option<io::Result<Loaded>>) {
        let shared = self.shared;
        let name = entry.file_name().as_bytes();
        let path = display(entry.path());
        let item = Entry {
            path,
            name_at: path.len().saturating_sub(name.len()),
            hit: job.hit,
            folder: false,
        };
        let named = job.named;
        self.quality = rank::quality(name, item.hit);
        let Some(pattern) = shared.plan.content().filter(|_| job.plain) else {
            if named {
                self.path_line(&item, true);
            }
            if job.muted && !shared.gather {
                self.weigh(entry);
            }
            return;
        };
        if named && matches!(shared.layout, Layout::Flat | Layout::List) {
            self.path_line(&item, true);
            if shared.layout == Layout::List {
                return;
            }
        }

        if shared.gather {
            let mut sink = Gather {
                matcher: pattern.lines(),
                seen: &mut self.scratch,
                fold: shared.fold,
                binary: false,
            };
            let searched = self
                .searcher
                .search_path(pattern.lines(), entry.path(), &mut sink);
            if sink.binary || searched.is_err() {
                self.scratch.clear();
            }
            spell::merge(&mut self.seen, &mut self.scratch);
            return;
        }

        let loaded = loaded.unwrap_or_else(|| self.load(entry.path(), true));
        let (headed, hits) = if pattern.single() {
            self.read(pattern.lines(), entry, &item, loaded)
        } else {
            self.read(pattern, entry, &item, loaded)
        };
        match shared.layout {
            Layout::List if hits > 0 => self.path_line(&item, false),
            Layout::Count if hits > 0 => self.count_line(&item, hits),
            Layout::Count if named => self.path_line(&item, true),
            Layout::Grouped if headed => self.out.push(b'\n'),
            Layout::Grouped if named => self.path_line(&item, true),
            _ => {}
        }
    }

    fn read<M: Matcher>(
        &mut self,
        matcher: &M,
        entry: &Item<'_>,
        item: &Entry<'_>,
        loaded: io::Result<Loaded>,
    ) -> (bool, u64) {
        let shared = self.shared;
        let start = self.out.len();
        let mut sink = Lines {
            out: &mut self.out,
            entry: item,
            matcher,
            layout: shared.layout,
            style: shared.style(),
            budget: &shared.budget,
            headed: false,
            hits: 0,
            first: None,
            binary: false,
        };
        let result = match loaded {
            Ok(Loaded::Text(length)) => {
                self.searcher
                    .search_slice(matcher, &self.loaded[..length], &mut sink)
            }
            Ok(Loaded::Binary) => Ok(()),
            Ok(Loaded::Large(rest)) => {
                let whole = self.loaded.as_slice().chain(rest);
                self.searcher.search_reader(matcher, whole, &mut sink)
            }
            Err(error) => Err(error),
        };
        let (mut headed, mut hits, first) = (sink.headed, sink.hits, sink.first);
        if sink.binary {
            self.out.truncate(start);
            (headed, hits) = (false, 0);
        }
        if let Err(error) = result {
            shared.fail(format_args!("{}: {error}", entry.path().display()));
        }
        self.tally.read += 1;
        if hits > 0 {
            shared.found.store(true, Ordering::Relaxed);
            self.line = first;
            self.tally.files += 1;
            if shared.layout != Layout::List {
                self.tally.lines += hits;
            }
        }
        (headed, hits)
    }

    fn path_line(&mut self, entry: &Entry<'_>, named: bool) {
        if self.shared.gather {
            if let Some((start, end)) = entry.hit {
                let name = &entry.path[entry.name_at.min(entry.path.len())..];
                spell::note(&mut self.seen, name, start, end, self.shared.fold);
            }
            return;
        }
        if !self.shared.budget.take() {
            return;
        }
        write_entry(&mut self.out, entry, self.shared.style());
        self.tally.names += u64::from(named);
        self.shared.found.store(true, Ordering::Relaxed);
    }

    fn count_line(&mut self, entry: &Entry<'_>, count: u64) {
        if self.shared.budget.take() {
            write_count(&mut self.out, entry, count, self.shared.style());
        }
    }

    fn in_scope(&mut self, entry: &Item<'_>, dir: &Pattern) -> bool {
        let Some(parent) = entry.path().parent() else {
            return false;
        };
        if self.scope.0 == parent.as_os_str() {
            return self.scope.1;
        }
        let inside = parent
            .components()
            .rev()
            .take(entry.depth().saturating_sub(1))
            .any(|part| match part {
                Component::Normal(name) => dir.find(name.as_bytes()).is_some(),
                _ => false,
            });
        self.scope = (parent.as_os_str().to_os_string(), inside);
        inside
    }

    fn passes_stat(&self, entry: &Item<'_>) -> bool {
        let Ok(metadata) = entry.metadata() else {
            return false;
        };
        let size = metadata.len();
        let sized = self.shared.filters.sizes.iter().all(|filter| {
            if filter.at_least {
                size >= filter.bytes
            } else {
                size <= filter.bytes
            }
        });
        if !sized || self.shared.cutoffs.is_empty() {
            return sized;
        }
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        self.shared.cutoffs.iter().all(|(older, cutoff)| {
            if *older {
                modified <= *cutoff
            } else {
                modified >= *cutoff
            }
        })
    }

    fn settle(&mut self) {
        if self.out.is_empty() {
            return;
        }
        if self.shared.live || self.out.len() >= CHUNK || self.flushed.elapsed() >= WAIT {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.out.is_empty() {
            return;
        }
        if stop::asked() {
            self.out.clear();
            return;
        }
        let written = io::stdout().lock().write_all(&self.out);
        self.out.clear();
        self.flushed = Instant::now();
        if written.is_err() {
            self.shared.budget.stop();
        }
    }
}

impl Drop for Worker<'_> {
    fn drop(&mut self) {
        while !self.waiting.is_empty() && !self.shared.budget.stopped() {
            self.next();
        }
        self.flush();
        if !self.seen.is_empty()
            && let Ok(mut seen) = self.shared.seen.lock()
        {
            spell::merge(&mut seen, &mut self.seen);
        }
        if !self.blocks.is_empty()
            && let Ok(mut blocks) = self.shared.blocks.lock()
        {
            blocks.append(&mut self.blocks);
        }
        if let Ok(mut tally) = self.shared.tally.lock() {
            tally.add(self.tally);
        }
    }
}

fn display(path: &Path) -> &[u8] {
    let bytes = path.as_os_str().as_bytes();
    bytes.strip_prefix(b"./").unwrap_or(bytes)
}

fn view(query: &Query) -> View {
    let output = &query.output;
    let terminal = io::stdout().is_terminal();
    let machine = output.json || output.null || output.acts() || output.browse;
    let pretty = !machine
        && match output.format {
            Format::Auto => terminal,
            Format::Pretty => true,
            Format::Plain => false,
        };
    let paths = output.list || output.best || output.why || output.null || output.acts();
    let layout = if output.browse && !output.list {
        Layout::Flat
    } else if paths {
        Layout::List
    } else if output.count {
        Layout::Count
    } else if pretty {
        Layout::Grouped
    } else {
        Layout::Flat
    };
    let color = pretty && std::env::var_os("NO_COLOR").is_none();
    View {
        layout,
        color,
        live: terminal,
        json: output.json && !output.acts() && !output.browse,
        end: if output.null { 0 } else { b'\n' },
        links: color
            && terminal
            && !output.no_links
            && std::env::var_os("SSH_CONNECTION").is_none(),
    }
}

fn excludes(query: &Query) -> Result<Override, String> {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut builder = OverrideBuilder::new(root);
    for exclude in &query.filters.excludes {
        let mut globs = vec![format!("!{exclude}")];
        if exclude.starts_with('.') && !exclude.contains(['/', '*', '?', '[']) {
            globs.push(format!("!*{exclude}"));
        }
        for glob in globs {
            builder
                .add(&glob)
                .map_err(|error| format!("-E {exclude} is not a usable pattern: {error}"))?;
        }
    }
    for glob in skips(query, Skip::Whole).globs {
        builder
            .add(&format!("!{glob}"))
            .map_err(|error| format!("-g {glob} is not a usable pattern: {error}"))?;
    }
    builder.build().map_err(|error| error.to_string())
}

fn skips(query: &Query, skip: Skip) -> Skips {
    if query.walk.no_ignore || query.walk.all {
        Skips::none()
    } else {
        Skips::new(&query.filters.groups, skip)
    }
}

fn walker(query: &Query) -> WalkBuilder {
    let hidden = query.walk.hidden || query.walk.all;
    let ignored = query.walk.no_ignore || query.walk.all;
    let mut builder = WalkBuilder::new(&query.roots[0]);
    for root in &query.roots[1..] {
        builder.add(root);
    }
    builder
        .hidden(!hidden)
        .ignore(!ignored)
        .git_ignore(!ignored)
        .git_global(!ignored)
        .git_exclude(!ignored)
        .parents(!ignored)
        .follow_links(query.walk.follow)
        .max_depth(query.walk.depth)
        .threads(threads(query));
    let git = hidden && !query.walk.all;
    let skips = skips(query, Skip::Whole);
    if git || skips.by_name() {
        builder.filter_entry(move |entry| {
            let name = entry.file_name().as_bytes();
            entry.depth() == 0 || !(git && name == b".git" || skips.matches(name))
        });
    }
    if let Ok(excluded) = excludes(query)
        && !excluded.is_empty()
    {
        builder.overrides(excluded);
    }
    builder
}

fn threads(query: &Query) -> usize {
    match query.walk.threads {
        0 => std::thread::available_parallelism().map_or(1, |cores| cores.get().min(WORKERS)),
        chosen => chosen,
    }
}

fn few(builder: &WalkBuilder) -> Option<Vec<Found>> {
    let started = Instant::now();
    let mut found = Vec::new();
    for result in builder.build() {
        if found.len() == FEW || started.elapsed() > PATIENCE {
            return None;
        }
        found.push(result);
    }
    Some(found)
}

fn handful(shared: &Shared, found: Vec<Found>, threads: usize) {
    let started = Instant::now();
    let mut found = found.into_iter();
    {
        let mut worker = Worker::new(shared, AHEAD);
        for result in found.by_ref() {
            if matches!(worker.visit(result), WalkState::Quit) {
                return;
            }
            if started.elapsed() > PATIENCE {
                break;
            }
        }
    }
    let crew = threads.min(found.len());
    let queue = Mutex::new(found);
    std::thread::scope(|scope| {
        for _ in 0..crew {
            scope.spawn(|| {
                let mut worker = Worker::new(shared, AHEAD);
                while let Some(result) = queue.lock().ok().and_then(|mut queue| queue.next()) {
                    if matches!(worker.visit(result), WalkState::Quit) {
                        break;
                    }
                }
            });
        }
    });
}

fn print(query: &Query) -> String {
    let whole = query
        .filters
        .groups
        .iter()
        .filter(|group| group.skip == Skip::Whole);
    let mut print = String::from("1");
    for pattern in whole.flat_map(|group| &group.patterns) {
        print.push('\n');
        print.push_str(pattern);
    }
    print
}

fn indexable(query: &Query) -> bool {
    let walk = &query.walk;
    !(walk.no_index || walk.hidden || walk.no_ignore || walk.all || walk.follow)
        && query.roots.len() == 1
}

fn stored(query: &Query, sorted: bool) -> (Option<Stored>, Advice) {
    if !indexable(query) {
        return (None, Advice::Leave);
    }
    let root = &query.roots[0];
    let index = match Index::open(root, &print(query)) {
        Opened::Ready(index) => index,
        Opened::Stale => return (None, Advice::Build),
        Opened::Missing => return (None, Advice::Large),
    };
    let bytes = root.as_os_str().as_bytes();
    let dirty = index.stale(bytes, threads(query));
    let limit = (index.folders() as usize / 16).max(64);
    if dirty.len() > limit {
        return (None, Advice::Build);
    }
    if dirty.is_empty() {
        let stored = Stored {
            index,
            dirty,
            fresh: None,
        };
        return (Some(stored), Advice::Leave);
    }
    if sorted {
        return (None, Advice::Leave);
    }
    match index.fresh(bytes, &dirty) {
        Some(fresh) => {
            let stored = Stored {
                index,
                dirty,
                fresh: Some(fresh),
            };
            (Some(stored), Advice::Leave)
        }
        None => (None, Advice::Build),
    }
}

fn refresh(shared: &Shared, query: &Query, fresh: Fresh) -> Vec<u32> {
    let fresh = Arc::new(fresh);
    let added: Arc<Mutex<HashSet<Vec<u8>>>> = Arc::default();
    let seen: Mutex<HashSet<u32>> = Mutex::default();
    let skips = skips(query, Skip::Whole);
    let mut builder = walker(query);
    builder.filter_entry({
        let (fresh, added) = (fresh.clone(), added.clone());
        move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if skips.matches(entry.file_name().as_bytes()) {
                return false;
            }
            let path = entry.path().as_os_str().as_bytes();
            let parent = entry
                .path()
                .parent()
                .map_or(&[][..], |parent| parent.as_os_str().as_bytes());
            fresh.dirty.contains_key(path)
                || fresh.above.contains(path)
                || fresh.dirty.contains_key(parent)
                || added.lock().is_ok_and(|added| added.contains(parent))
        }
    });
    builder.build_parallel().run(|| {
        let mut worker = Worker::new(shared, AHEAD);
        let (fresh, added, seen) = (&fresh, &added, &seen);
        Box::new(move |result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    shared.fail(error);
                    return WalkState::Continue;
                }
            };
            let path = entry.path().as_os_str().as_bytes();
            if entry.file_type().is_some_and(|kind| kind.is_dir()) {
                let known = fresh.dirty.contains_key(path) || fresh.above.contains(path);
                if let Some(slot) = fresh.kids.get(path) {
                    if let Ok(mut seen) = seen.lock() {
                        seen.insert(*slot);
                    }
                    return match known {
                        true => WalkState::Continue,
                        false => WalkState::Skip,
                    };
                }
                if known || entry.depth() == 0 {
                    return WalkState::Continue;
                }
                if let Ok(mut added) = added.lock() {
                    added.insert(path.to_vec());
                }
            }
            worker.visit(Ok(entry))
        })
    });
    let seen = seen.into_inner().unwrap_or_default();
    fresh
        .kids
        .values()
        .filter(|slot| !seen.contains(slot))
        .copied()
        .collect()
}

fn replay(shared: &Shared, query: &Query, stored: Stored, sorted: bool) {
    let index = &stored.index;
    let mut state = vec![0u8; index.folders() as usize];
    for slot in &stored.dirty {
        state[*slot as usize] |= index::DIRTY;
    }
    if let Some(fresh) = stored.fresh {
        for slot in refresh(shared, query, fresh) {
            state[slot as usize] |= index::GONE;
        }
    }
    let excluded = excludes(query).ok().filter(|excluded| !excluded.is_empty());
    let scan = Scan {
        index,
        root: query.roots[0].as_os_str().as_bytes(),
        depth: query.walk.depth,
        state: &state,
        excluded: excluded.as_ref(),
    };
    let total = index.entries();
    if sorted {
        let mut worker = Worker::new(shared, 0);
        scan.run(0..total, &mut |visit| worker.take(visit));
        return;
    }
    let next = AtomicU32::new(0);
    std::thread::scope(|scope| {
        for _ in 0..threads(query) {
            scope.spawn(|| {
                let mut worker = Worker::new(shared, AHEAD);
                let mut going = true;
                while going && !shared.budget.stopped() {
                    let from = next.fetch_add(PIECE, Ordering::Relaxed);
                    if from >= total {
                        break;
                    }
                    let piece = from..from.saturating_add(PIECE).min(total);
                    scan.run(piece, &mut |visit| {
                        going = worker.take(visit);
                        going
                    });
                }
            });
        }
    });
}

fn search(query: &Query, plan: Compiled, view: View, pass: Pass, keep: bool) -> Outcome {
    let now = SystemTime::now();
    let cutoffs = query
        .filters
        .ages
        .iter()
        .map(|age| {
            let cutoff = now
                .checked_sub(Duration::from_secs(age.seconds))
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (age.older, cutoff)
        })
        .collect();
    let gather = matches!(pass, Pass::Gather { .. });
    let shared = Shared {
        plan,
        folders: matches!(query.filters.kind, None | Some(Kind::Dir))
            && !query.filters.files_only(),
        stat: !query.filters.sizes.is_empty() || !query.filters.ages.is_empty(),
        cutoffs,
        layout: if gather { Layout::Flat } else { view.layout },
        color: view.color,
        json: view.json,
        end: view.end,
        base: view.links.then(rank::here),
        live: view.live,
        context: if matches!(view.layout, Layout::List | Layout::Count)
            || gather
            || query.output.browse
        {
            0
        } else {
            query.output.context
        },
        budget: Budget::new(if gather || keep {
            None
        } else {
            query.output.max
        }),
        found: AtomicBool::new(false),
        failed: AtomicBool::new(false),
        quiet: pass != Pass::First,
        gather,
        fold: matches!(pass, Pass::Gather { fold: true }),
        seen: Mutex::new(Seen::new()),
        keep: keep && !gather,
        blocks: Mutex::new(Vec::new()),
        tally: Mutex::new(Tally::default()),
        filters: query.filters.clone(),
        unread: skips(query, Skip::Contents),
        follow: query.walk.follow,
    };

    let sorted = query.walk.sort && !gather;
    let (stored, advice) = stored(query, sorted);
    let reread = stored.as_ref().map(|stored| stored.dirty.len());
    let mut builder = walker(query);
    if let Some(stored) = stored {
        replay(&shared, query, stored, sorted);
    } else if sorted {
        builder.sort_by_file_name(|left, right| left.cmp(right));
        let mut worker = Worker::new(&shared, 0);
        for result in builder.build() {
            if matches!(worker.visit(result), WalkState::Quit) {
                break;
            }
        }
    } else {
        match few(&builder) {
            Some(found) => handful(&shared, found, threads(query)),
            None => builder.build_parallel().run(|| {
                let mut worker = Worker::new(&shared, AHEAD);
                Box::new(move |result| worker.visit(result))
            }),
        }
    }

    Outcome {
        found: shared.found.load(Ordering::Relaxed),
        failed: shared.failed.load(Ordering::Relaxed),
        seen: shared.seen.into_inner().unwrap_or_default(),
        blocks: shared.blocks.into_inner().unwrap_or_default(),
        tally: shared.tally.into_inner().unwrap_or_default(),
        advice,
        reread,
    }
}

pub fn ranked(query: &Query, terminal: bool) -> bool {
    let names_only = match &query.plan {
        Plan::Union {
            pattern: Some(_),
            targets,
        } => !targets.content,
        Plan::Union { pattern: None, .. } => false,
        Plan::Narrow { content, .. } => content.is_none(),
    };
    if query.output.best || query.output.why || query.output.open || query.output.browse {
        true
    } else if query.walk.sort {
        false
    } else if query.output.exec.is_some() {
        query.output.rank == Some(true)
    } else {
        query.output.rank.unwrap_or(terminal && names_only)
    }
}

fn remembered(query: &Query, habits: &Habits, found: &[Block], view: View) -> Vec<Block> {
    let mut extra = Vec::new();
    let Plan::Union {
        pattern: Some(source),
        targets,
    } = &query.plan
    else {
        return extra;
    };
    if query.filters.files_only() {
        return extra;
    }
    let Ok(pattern) = Pattern::compile(source, &query.matching) else {
        return extra;
    };
    let base = view.links.then(rank::here);
    let style = Style {
        color: view.color,
        json: view.json,
        end: view.end,
        base: base.as_deref(),
    };
    let excluded = excludes(query).ok().filter(|excluded| !excluded.is_empty());
    let skipped = skips(query, Skip::Whole);
    let roots: Vec<Vec<u8>> = query
        .roots
        .iter()
        .map(|root| habits.absolute(root.as_os_str().as_bytes()))
        .collect();
    let mut have: HashSet<Vec<u8>> = found
        .iter()
        .map(|block| habits.absolute(&block.path))
        .collect();
    for path in habits.known() {
        let below = roots.iter().find_map(|root| {
            let rest = path.strip_prefix(root.as_slice())?;
            let rest = if root.len() > 1 {
                rest.strip_prefix(b"/")?
            } else {
                rest
            };
            (!rest.is_empty()).then_some(rest)
        });
        let Some(below) = below else {
            continue;
        };
        let depth = below.split(|byte| *byte == b'/').count();
        if query.walk.depth.is_some_and(|deepest| depth > deepest)
            || have.contains(path)
            || below
                .split(|byte| *byte == b'/')
                .any(|part| skipped.matches(part))
        {
            continue;
        }
        let name = below.rsplit(|byte| *byte == b'/').next().unwrap_or(below);
        let Some(hit) = pattern.find(name) else {
            continue;
        };
        let Ok(metadata) = fs::metadata(OsStr::from_bytes(path)) else {
            continue;
        };
        let folder = metadata.is_dir();
        let wanted = match query.filters.kind {
            None => folder || metadata.is_file(),
            Some(Kind::Dir) => folder,
            Some(Kind::File) => metadata.is_file(),
            Some(Kind::Link) => false,
        };
        if !wanted || !(if folder { targets.dirs } else { targets.files }) {
            continue;
        }
        let shown = habits.relative(path);
        if let Some(excluded) = &excluded
            && Path::new(OsStr::from_bytes(shown))
                .ancestors()
                .filter(|part| !part.as_os_str().is_empty())
                .enumerate()
                .any(|(above, part)| excluded.matched(part, above > 0 || folder).is_ignore())
        {
            continue;
        }
        let mut text = Vec::new();
        let entry = Entry {
            path: shown,
            name_at: shown.len().saturating_sub(name.len()),
            hit: Some(hit),
            folder,
        };
        write_entry(&mut text, &entry, style);
        have.insert(path.to_vec());
        extra.push(Block {
            path: shown.to_vec(),
            quality: rank::quality(name, Some(hit)),
            depth,
            modified: metadata.modified().ok(),
            line: None,
            text,
        });
    }
    extra
}

fn arrange(blocks: Vec<Block>, habits: Option<&Habits>, query: &Query) -> Vec<(Score, Block)> {
    let mut blocks = match habits {
        Some(habits) => habits.order(blocks),
        None => {
            let mut blocks: Vec<(Score, Block)> = blocks
                .into_iter()
                .map(|block| (Score::default(), block))
                .collect();
            blocks.sort_by(|left, right| left.1.path.cmp(&right.1.path));
            blocks
        }
    };
    let single = (query.output.best || query.output.open) && !query.output.browse;
    if let Some(keep) = single.then_some(1).or(query.output.max) {
        blocks.truncate(keep);
    }
    if single
        && !query.output.why
        && let (Some(habits), Some((_, top))) = (habits, blocks.first())
    {
        habits.remember(&top.path);
    }
    blocks
}

fn show(mut blocks: Vec<(Score, Block)>, query: &Query, view: View) {
    let why = query.output.why;
    if view.live {
        blocks.reverse();
    }
    let mut stdout = io::stdout().lock();
    let mut chunk: Vec<u8> = Vec::with_capacity(CHUNK);
    if why && !blocks.is_empty() {
        chunk.extend_from_slice(rank::Score::heading().as_bytes());
    }
    for (score, block) in &blocks {
        if why {
            chunk.extend_from_slice(score.row().as_bytes());
        }
        chunk.extend_from_slice(&block.text);
        if chunk.len() >= CHUNK {
            if stdout.write_all(&chunk).is_err() {
                return;
            }
            chunk.clear();
        }
    }
    let _ = stdout.write_all(&chunk);
}

fn grouped(number: u64) -> String {
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

fn counted(number: u64, one: &str, many: &str) -> String {
    format!(
        "{} {}",
        grouped(number),
        if number == 1 { one } else { many }
    )
}

fn totals(tally: Tally, layout: Layout, elapsed: Duration, reread: Option<usize>) -> String {
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

pub fn suggest(query: &Query) -> Option<String> {
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

fn reindex(mut query: Query, started: Instant) -> ExitCode {
    let query = &mut query;
    let plain = indexable(query) && query.filters.excludes.is_empty() && query.walk.depth.is_none();
    if !plain {
        eprintln!(
            "syk: an index holds what a plain search of one folder sees, so -y does not go with -H, -I, -a, -L, -E, -D, -Y or several paths."
        );
        return ExitCode::from(2);
    }
    let shown = query.roots[0].clone();
    if let Ok(real) = fs::canonicalize(&shown) {
        query.roots[0] = real;
    }
    let root = &query.roots[0];
    match index::build(root, &print(query), walker(query)) {
        Ok(built) => {
            println!(
                "Indexed {} in {} below {} in {:.2} s.",
                counted(built.entries as u64, "entry", "entries"),
                counted(built.folders as u64, "folder", "folders"),
                shown.display(),
                started.elapsed().as_secs_f64()
            );
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("syk: no index for {}: {reason}.", shown.display());
            ExitCode::from(2)
        }
    }
}

fn background(query: &Query) {
    let print = print(query);
    let Ok(root) = fs::canonicalize(&query.roots[0]) else {
        return;
    };
    if !index::wanted(&root, &print) {
        return;
    }
    let mut flags = vec!["-U".to_string(), "-u".to_string(), group::EVERY.to_string()];
    let whole = query
        .filters
        .groups
        .iter()
        .filter(|group| group.skip == Skip::Whole);
    for group in whole {
        flags.push("-g".to_string());
        flags.push(format!("{}={}", group.name, group.patterns.join(",")));
    }
    let mut folder = root.into_os_string();
    folder.push("/");
    let _ = index::spawn(Path::new(&folder), &flags);
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
