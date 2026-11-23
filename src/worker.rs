use std::collections::VecDeque;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use grep_matcher::Matcher;
use grep_searcher::sinks::Bytes;
use grep_searcher::{BinaryDetection, Searcher, SearcherBuilder};
use ignore::{DirEntry, WalkState};

use crate::group::{Skips, ends_with};
use crate::index::Visit;
use crate::item::{Form, Item};
use crate::load::{self, Loaded};
use crate::need::Need;
use crate::output::{Budget, Entry, Layout, Lines, Style, write_count, write_entry};
use crate::pattern::Pattern;
use crate::plan::Compiled;
use crate::query::{Filters, Kind};
use crate::rank::{self, Block};
use crate::spell::{self, Gather, Seen};
use crate::stop;

pub const CHUNK: usize = 64 * 1024;
const WAIT: Duration = Duration::from_millis(10);
pub const AHEAD: usize = 32;

pub struct Shared {
    pub plan: Compiled,
    pub filters: Filters,
    pub unread: Skips,
    pub follow: bool,
    pub folders: bool,
    pub stat: bool,
    pub cutoffs: Vec<(bool, SystemTime)>,
    pub layout: Layout,
    pub color: bool,
    pub json: bool,
    pub end: u8,
    pub base: Option<Vec<u8>>,
    pub live: bool,
    pub context: usize,
    pub budget: Budget,
    pub found: AtomicBool,
    pub failed: AtomicBool,
    pub quiet: bool,
    pub gather: bool,
    pub fold: bool,
    pub seen: Mutex<Seen>,
    pub keep: bool,
    pub blocks: Mutex<Vec<Block>>,
    pub tally: Mutex<Tally>,
}

#[derive(Clone, Copy, Default)]
pub struct Tally {
    pub entries: u64,
    pub read: u64,
    pub lines: u64,
    pub files: u64,
    pub names: u64,
    pub spared: u64,
    pub withheld: Withheld,
}

#[derive(Clone, Copy, Default)]
pub struct Withheld {
    pub files: u64,
    pub lines: u64,
}

impl Tally {
    fn add(&mut self, other: Tally) {
        self.entries += other.entries;
        self.read += other.read;
        self.lines += other.lines;
        self.files += other.files;
        self.names += other.names;
        self.spared += other.spared;
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

    pub fn gate(&self) -> Option<&Pattern> {
        let pattern = match &self.plan {
            Compiled::Union {
                pattern: Some(pattern),
                targets,
            } if !targets.content => pattern,
            Compiled::Narrow {
                name: Some(name), ..
            } => name,
            _ => return None,
        };
        (!self.gather && pattern.siftable()).then_some(pattern)
    }

    pub fn need(&self) -> Option<Need> {
        let content = self.plan.content().filter(|_| !self.gather)?;
        Some(content.need())
    }

    pub fn fail(&self, message: impl std::fmt::Display) {
        self.failed.store(true, Ordering::Relaxed);
        if !self.quiet {
            eprintln!("syk: {message}");
        }
    }
}

pub type Found = Result<DirEntry, ignore::Error>;

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

pub struct Worker<'a> {
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
    pub fn new(shared: &'a Shared, ahead: usize) -> Worker<'a> {
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

    pub fn visit(&mut self, result: Found) -> WalkState {
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
            ruled: None,
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
            let spared = job.muted && !self.shared.gather && self.spared(entry, true);
            self.handle(entry, job, spared.then_some(Ok(Loaded::Unmatched)));
            return WalkState::Continue;
        }
        if self.spared(entry, false) {
            self.handle(entry, job, Some(Ok(Loaded::Unmatched)));
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

    pub fn take(&mut self, visit: Visit<'_>) -> bool {
        match visit {
            Visit::Entry(entry) => !matches!(self.see(&entry), WalkState::Quit),
            Visit::Failure(text) => {
                self.shared.fail(text);
                true
            }
            Visit::Passed(entries) => {
                self.tally.entries += entries;
                true
            }
        }
    }

    fn spared(&mut self, entry: &Item<'_>, muted: bool) -> bool {
        let spared = entry
            .ruled
            .is_some_and(|ruled| !(muted && ruled.binary) && ruled.holds(entry.path()));
        self.tally.spared += u64::from(spared);
        spared
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
            ruled: None,
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
            let spared = matches!(loaded, Some(Ok(Loaded::Unmatched)));
            if job.muted && !shared.gather && !spared {
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
        let spared = matches!(loaded, Ok(Loaded::Unmatched));
        let result = match loaded {
            Ok(Loaded::Text(length)) => {
                self.searcher
                    .search_slice(matcher, &self.loaded[..length], &mut sink)
            }
            Ok(Loaded::Binary | Loaded::Unmatched) => Ok(()),
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
        self.tally.read += u64::from(!spared);
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

#[cfg(target_os = "linux")]
impl crate::crawl::Sink for Worker<'_> {
    fn take(&mut self, visit: Visit<'_>) -> bool {
        Worker::take(self, visit)
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
