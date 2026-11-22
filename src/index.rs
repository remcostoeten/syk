use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File, Metadata};
use std::io::{self, BufRead, BufReader, Write};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ignore::WalkBuilder;
use ignore::overrides::Override;
use memmap2::Mmap;

use crate::item::{Form, Item};

const MAGIC: &[u8; 8] = b"sykidx01";
const HEAD: usize = 48;
const RECORD: usize = 20;
const FOLDER: usize = 40;
const STAMP: usize = 32;
const SLACK: i64 = 2_000_000_000;
const RACY: u32 = 1;
const GITIGNORE: u32 = 2;
const IGNORE: u32 = 4;
const GIT: u32 = 8;
const JJ: u32 = 16;
const PROBES: u32 = GITIGNORE | IGNORE | GIT | JJ;
const BELOW: u8 = 1;
const BARE: u8 = 2;
const STRIDE: u32 = 512;
const KEPT: usize = 24;
const LOCKED: Duration = Duration::from_secs(600);
const REFUSED: Duration = Duration::from_secs(24 * 60 * 60);

pub const DIRTY: u8 = 1;
pub const GONE: u8 = 2;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Stamp {
    modified: i64,
    changed: i64,
    inode: u64,
    device: u64,
}

impl Stamp {
    fn of(metadata: &Metadata) -> Stamp {
        Stamp {
            modified: metadata
                .mtime()
                .saturating_mul(1_000_000_000)
                .saturating_add(metadata.mtime_nsec()),
            changed: metadata
                .ctime()
                .saturating_mul(1_000_000_000)
                .saturating_add(metadata.ctime_nsec()),
            inode: metadata.ino(),
            device: metadata.dev(),
        }
    }

    fn recent(&self, built: i64) -> bool {
        self.modified >= built - SLACK || self.changed >= built - SLACK
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend(self.modified.to_le_bytes());
        out.extend(self.changed.to_le_bytes());
        out.extend(self.inode.to_le_bytes());
        out.extend(self.device.to_le_bytes());
    }

    fn read(bytes: &[u8]) -> Stamp {
        Stamp {
            modified: long(bytes, 0) as i64,
            changed: long(bytes, 8) as i64,
            inode: long(bytes, 16),
            device: long(bytes, 24),
        }
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    bytes
        .get(at..at + 4)
        .and_then(|part| part.try_into().ok())
        .map_or(0, u32::from_le_bytes)
}

fn short(bytes: &[u8], at: usize) -> u16 {
    bytes
        .get(at..at + 2)
        .and_then(|part| part.try_into().ok())
        .map_or(0, u16::from_le_bytes)
}

fn long(bytes: &[u8], at: usize) -> u64 {
    bytes
        .get(at..at + 8)
        .and_then(|part| part.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as i64)
}

#[derive(Clone, Copy, Default)]
struct Record {
    name: u32,
    parent: u32,
    end: u32,
    len: u16,
    depth: u16,
    tag: u32,
}

impl Record {
    fn form(&self) -> Form {
        match self.tag & 3 {
            0 => Form::File,
            1 => Form::Dir,
            2 => Form::Link,
            _ => Form::Other,
        }
    }

    fn slot(&self) -> u32 {
        self.tag >> 2
    }

    fn tag(form: Form, slot: u32) -> u32 {
        let kind = match form {
            Form::File => 0,
            Form::Dir => 1,
            Form::Link => 2,
            Form::Other => 3,
        };
        slot << 2 | kind
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend(self.name.to_le_bytes());
        out.extend(self.parent.to_le_bytes());
        out.extend(self.end.to_le_bytes());
        out.extend(self.len.to_le_bytes());
        out.extend(self.depth.to_le_bytes());
        out.extend(self.tag.to_le_bytes());
    }
}

#[derive(Clone, Copy, Default)]
struct Folder {
    entry: u32,
    flags: u32,
    stamp: Stamp,
}

#[derive(Clone, PartialEq, Eq)]
struct Watched {
    path: Vec<u8>,
    mode: u8,
    kind: u8,
    stamp: Stamp,
}

impl Watched {
    fn see(path: &Path, mode: u8) -> (u8, Stamp) {
        let (kind, stamp) = match fs::metadata(path) {
            Ok(metadata) if metadata.is_dir() => (1, Stamp::of(&metadata)),
            Ok(metadata) => (2, Stamp::of(&metadata)),
            Err(_) => (0, Stamp::default()),
        };
        match mode & BARE {
            0 => (kind, stamp),
            _ => (kind, Stamp::default()),
        }
    }

    fn new(path: &Path, stored: &Path, mode: u8) -> Watched {
        let (kind, stamp) = Watched::see(path, mode);
        Watched {
            path: stored.as_os_str().as_bytes().to_vec(),
            mode,
            kind,
            stamp,
        }
    }
}

fn below(root: &[u8], rest: &[u8]) -> Vec<u8> {
    let mut path = root.to_vec();
    if !rest.is_empty() {
        if !path.ends_with(b"/") {
            path.push(b'/');
        }
        path.extend(rest);
    }
    path
}

fn probe(dir: &Path) -> u32 {
    [
        (".gitignore", GITIGNORE),
        (".ignore", IGNORE),
        (".git", GIT),
        (".jj", JJ),
    ]
    .into_iter()
    .filter(|(name, _)| dir.join(name).exists())
    .fold(0, |bits, (_, bit)| bits | bit)
}

fn first_line(path: &Path) -> Option<String> {
    BufReader::new(File::open(path).ok()?).lines().next()?.ok()
}

fn git(dir: &Path) -> Vec<PathBuf> {
    let dot = dir.join(".git");
    let Ok(metadata) = fs::metadata(&dot) else {
        return Vec::new();
    };
    if metadata.is_dir() {
        return vec![dot.join("info/exclude")];
    }
    let mut paths = vec![dot.clone()];
    let Some(real) = first_line(&dot)
        .as_deref()
        .and_then(|line| line.strip_prefix("gitdir: "))
        .map(PathBuf::from)
    else {
        return paths;
    };
    let common = real.join("commondir");
    paths.push(common.clone());
    if let Some(line) = first_line(&common) {
        let shared = if line.starts_with('.') {
            real.join(line)
        } else {
            PathBuf::from(line)
        };
        paths.push(shared.join("info/exclude"));
    }
    paths
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn outer(root: &Path) -> Vec<Watched> {
    let mut paths: Vec<(PathBuf, u8)> = Vec::new();
    if let Some(chosen) = std::env::var_os("GIT_CONFIG_GLOBAL") {
        paths.push((PathBuf::from(chosen), 0));
    }
    if let Some(home) = home() {
        paths.push((home.join(".gitconfig"), 0));
    }
    let settings = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|folder| !folder.is_empty())
        .map(PathBuf::from)
        .or_else(|| home().map(|home| home.join(".config")));
    if let Some(settings) = settings {
        paths.push((settings.join("git/config"), 0));
        paths.push((settings.join("git/ignore"), 0));
    }
    if let Some(excludes) = ignore::gitignore::gitconfig_excludes_path() {
        paths.push((excludes, 0));
    }
    for above in root.ancestors().skip(1) {
        paths.push((above.join(".gitignore"), 0));
        paths.push((above.join(".ignore"), 0));
        paths.push((above.join(".git"), BARE));
        paths.push((above.join(".jj"), BARE));
        paths.extend(git(above).into_iter().map(|path| (path, 0)));
    }
    paths
        .iter()
        .map(|(path, mode)| Watched::new(path, path, *mode))
        .collect()
}

#[cfg(target_os = "linux")]
fn trusted(path: &Path) -> bool {
    // ext, btrfs, xfs, tmpfs, f2fs, zfs and bcachefs update a folder's times on every change
    const LOCAL: [i64; 7] = [
        0xEF53,
        0x9123_683E,
        0x5846_5342,
        0x0102_1994,
        0xF2F5_2010,
        0x2FC1_2FC1,
        0xCA45_1A4E,
    ];
    let Ok(text) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut found: libc::statfs = unsafe { std::mem::zeroed() };
    let status = unsafe { libc::statfs(text.as_ptr(), &mut found) };
    status == 0 && LOCAL.contains(&(found.f_type as i64 & 0xFFFF_FFFF))
}

#[cfg(not(target_os = "linux"))]
fn trusted(_: &Path) -> bool {
    false
}

fn store() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .or_else(|| home().map(|home| home.join(".cache")))?;
    Some(cache.join("syk").join("index"))
}

fn key(root: &Path, print: &str) -> String {
    // FNV-1a, so the file name stays the same between versions of the compiler
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let parts = [root.as_os_str().as_bytes(), &[0], print.as_bytes()];
    for byte in parts.into_iter().flatten() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn place(root: &Path, print: &str, kind: &str) -> Option<PathBuf> {
    Some(store()?.join(format!("{}.{kind}", key(root, print))))
}

fn younger(path: &Path, limit: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < limit)
}

struct Trail {
    path: Vec<u8>,
    lens: Vec<usize>,
}

impl Trail {
    fn new() -> Trail {
        Trail {
            path: Vec::with_capacity(256),
            lens: Vec::with_capacity(32),
        }
    }

    fn step(&mut self, root: &[u8], depth: usize, name: &[u8]) {
        if depth == 0 {
            self.path.clear();
            self.path.extend(root);
            self.lens.clear();
            self.lens.push(self.path.len());
            return;
        }
        let parent = self.lens.get(depth - 1).copied().unwrap_or(0);
        self.path.truncate(parent);
        if !self.path.ends_with(b"/") {
            self.path.push(b'/');
        }
        self.path.extend(name);
        self.lens.truncate(depth);
        self.lens.push(self.path.len());
    }

    fn as_path(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.path))
    }
}

pub struct Index {
    map: Mmap,
    built: i64,
    entries: u32,
    folders: u32,
    outside: u32,
    root: Range<usize>,
    print: Range<usize>,
    records: usize,
    slots: usize,
    names: Range<usize>,
    context: Range<usize>,
    failures: Vec<(u32, Range<usize>)>,
}

pub enum Opened {
    Ready(Index),
    Stale,
    Missing,
}

pub enum Visit<'a> {
    Entry(Item<'a>),
    Failure(&'a str),
}

pub struct Scan<'a> {
    pub index: &'a Index,
    pub root: &'a [u8],
    pub depth: Option<usize>,
    pub state: &'a [u8],
    pub excluded: Option<&'a Override>,
}

pub struct Fresh {
    pub dirty: HashMap<Vec<u8>, u32>,
    pub above: HashSet<Vec<u8>>,
    pub kids: HashMap<Vec<u8>, u32>,
}

impl Index {
    fn load(path: &Path) -> Option<Index> {
        let file = File::open(path).ok()?;
        // The file is replaced by a rename and never changed in place
        let map = unsafe { Mmap::map(&file) }.ok()?;
        if map.len() < HEAD || &map[..8] != MAGIC {
            return None;
        }
        let built = long(&map, 8) as i64;
        let count = |at: usize| word(&map, 16 + at * 4);
        let (entries, folders, outside) = (count(0), count(1), count(7));
        let mut at = HEAD;
        let mut take = |size: usize| {
            let part = at..at.checked_add(size)?;
            at = part.end;
            (part.end <= map.len()).then_some(part)
        };
        let root = take(count(3) as usize)?;
        let print = take(count(4) as usize)?;
        let records = take((entries as usize).checked_mul(RECORD)?)?.start;
        let slots = take((folders as usize).checked_mul(FOLDER)?)?.start;
        let names = take(count(2) as usize)?;
        let context = take(count(5) as usize)?;
        let errors = take(count(6) as usize)?;
        let mut failures = Vec::new();
        let mut at = errors.start;
        while at + 6 <= errors.end {
            let text = at + 6..at + 6 + short(&map, at + 4) as usize;
            if text.end > errors.end {
                return None;
            }
            failures.push((word(&map, at), text.clone()));
            at = text.end;
        }
        (entries > 0 && folders > 0).then_some(Index {
            map,
            built,
            entries,
            folders,
            outside,
            root,
            print,
            records,
            slots,
            names,
            context,
            failures,
        })
    }

    pub fn open(root: &Path, print: &str) -> Opened {
        let folder = fs::symlink_metadata(root).is_ok_and(|found| found.is_dir());
        let Some(real) = folder.then(|| fs::canonicalize(root).ok()).flatten() else {
            return Opened::Missing;
        };
        let Some(index) = place(&real, print, "idx").and_then(|path| Index::load(&path)) else {
            return Opened::Missing;
        };
        let same = &index.map[index.root.clone()] == real.as_os_str().as_bytes()
            && &index.map[index.print.clone()] == print.as_bytes();
        match same && trusted(&real) && index.settled(&real, root) {
            true => Opened::Ready(index),
            false => Opened::Stale,
        }
    }

    pub fn entries(&self) -> u32 {
        self.entries
    }

    pub fn folders(&self) -> u32 {
        self.folders
    }

    fn record(&self, at: u32) -> Record {
        let start = self.records + at as usize * RECORD;
        match self.map.get(start..start + RECORD) {
            Some(bytes) if at < self.entries => Record {
                name: word(bytes, 0),
                parent: word(bytes, 4),
                end: word(bytes, 8),
                len: short(bytes, 12),
                depth: short(bytes, 14),
                tag: word(bytes, 16),
            },
            _ => Record::default(),
        }
    }

    fn name(&self, record: &Record) -> &[u8] {
        let start = self.names.start + record.name as usize;
        self.map
            .get(start..start + record.len as usize)
            .filter(|name| start + name.len() <= self.names.end)
            .unwrap_or(&[])
    }

    fn folder(&self, slot: u32) -> Folder {
        let start = self.slots + slot as usize * FOLDER;
        match self.map.get(start..start + FOLDER) {
            Some(bytes) if slot < self.folders => Folder {
                entry: word(bytes, 0),
                flags: word(bytes, 4),
                stamp: Stamp::read(&bytes[8..]),
            },
            _ => Folder::default(),
        }
    }

    fn watched(&self) -> Vec<Watched> {
        let bytes = &self.map[self.context.clone()];
        let mut watched = Vec::new();
        let mut at = 0;
        while at + 4 + STAMP <= bytes.len() {
            let end = at + 4 + short(bytes, at) as usize;
            if end + STAMP > bytes.len() {
                break;
            }
            watched.push(Watched {
                path: bytes[at + 4..end].to_vec(),
                mode: bytes[at + 2],
                kind: bytes[at + 3],
                stamp: Stamp::read(&bytes[end..]),
            });
            at = end + STAMP;
        }
        watched
    }

    fn settled(&self, real: &Path, root: &Path) -> bool {
        let stored = self.watched();
        let outside = self.outside as usize;
        if stored.len() < outside || stored[..outside] != outer(real)[..] {
            return false;
        }
        let root = root.as_os_str().as_bytes();
        let inside = stored[outside..].iter().all(|watched| {
            let place = match watched.mode & BELOW {
                0 => watched.path.clone(),
                _ => below(root, &watched.path),
            };
            let seen = Watched::see(Path::new(OsStr::from_bytes(&place)), watched.mode);
            seen == (watched.kind, watched.stamp)
        });
        inside
            && !stored
                .iter()
                .any(|watched| watched.stamp.recent(self.built))
    }

    fn chain(&self, at: u32) -> Vec<u32> {
        let mut chain = Vec::new();
        let mut record = self.record(at);
        let mut current = at;
        while record.depth > 0 && record.parent < current {
            current = record.parent;
            record = self.record(current);
            chain.push(current);
        }
        chain.reverse();
        chain
    }

    fn check(&self, root: &[u8], slots: Range<u32>, dirty: &mut Vec<u32>) {
        let mut trail = Trail::new();
        for above in self.chain(self.folder(slots.start).entry) {
            let record = self.record(above);
            trail.step(root, record.depth as usize, self.name(&record));
        }
        for slot in slots {
            let folder = self.folder(slot);
            let record = self.record(folder.entry);
            trail.step(root, record.depth as usize, self.name(&record));
            let clean = folder.flags & RACY == 0
                && fs::symlink_metadata(trail.as_path())
                    .is_ok_and(|found| found.is_dir() && Stamp::of(&found) == folder.stamp);
            if !clean {
                dirty.push(slot);
            }
        }
    }

    pub fn stale(&self, root: &[u8], threads: usize) -> Vec<u32> {
        let next = AtomicU32::new(0);
        let mut dirty: Vec<u32> = std::thread::scope(|scope| {
            let crew: Vec<_> = (0..threads.max(1))
                .map(|_| {
                    scope.spawn(|| {
                        let mut dirty = Vec::new();
                        loop {
                            let from = next.fetch_add(STRIDE, Ordering::Relaxed);
                            if from >= self.folders {
                                return dirty;
                            }
                            let to = from.saturating_add(STRIDE).min(self.folders);
                            self.check(root, from..to, &mut dirty);
                        }
                    })
                })
                .collect();
            crew.into_iter()
                .flat_map(|worker| worker.join().unwrap_or_default())
                .collect()
        });
        dirty.sort_unstable();
        dirty
    }

    pub fn fresh(&self, root: &[u8], dirty: &[u32]) -> Option<Fresh> {
        let mut fresh = Fresh {
            dirty: HashMap::new(),
            above: HashSet::new(),
            kids: HashMap::new(),
        };
        let mut trail = Trail::new();
        for slot in dirty {
            let folder = self.folder(*slot);
            let record = self.record(folder.entry);
            for above in self.chain(folder.entry) {
                let record = self.record(above);
                trail.step(root, record.depth as usize, self.name(&record));
                fresh.above.insert(trail.path.clone());
            }
            trail.step(root, record.depth as usize, self.name(&record));
            if probe(trail.as_path()) != folder.flags & PROBES {
                return None;
            }
            fresh.dirty.insert(trail.path.clone(), *slot);
            let mut child = folder.entry + 1;
            while child < record.end.min(self.entries) {
                let inner = self.record(child);
                if inner.form() != Form::Dir {
                    child += 1;
                    continue;
                }
                fresh
                    .kids
                    .insert(below(&trail.path, self.name(&inner)), inner.slot());
                child = inner.end.max(child + 1);
            }
        }
        for path in fresh.dirty.keys() {
            fresh.above.remove(path);
        }
        Some(fresh)
    }

    fn failure(&self, text: Range<usize>, root: &[u8]) -> String {
        let stored = String::from_utf8_lossy(&self.map[text]);
        let real = String::from_utf8_lossy(&self.map[self.root.clone()]);
        let given = String::from_utf8_lossy(root);
        let inside = format!("{}/", real.trim_end_matches('/'));
        match stored.strip_prefix(real.as_ref()) {
            Some(rest) if !stored.starts_with(&inside) => format!("{given}{rest}"),
            _ => stored.replace(&inside, &format!("{}/", given.trim_end_matches('/'))),
        }
    }

    fn failures(&self, from: u32) -> usize {
        self.failures.partition_point(|(entry, _)| *entry < from)
    }
}

impl Scan<'_> {
    fn skipped(&self, record: &Record, path: &Path) -> bool {
        let depth = record.depth as usize;
        let folder = record.form() == Form::Dir;
        let state = self.state.get(record.slot() as usize).copied().unwrap_or(0);
        self.depth.is_some_and(|deepest| depth > deepest)
            || (folder && state & GONE != 0)
            || (!folder && depth > 0 && state & DIRTY != 0)
            || (depth > 0
                && self
                    .excluded
                    .is_some_and(|excluded| excluded.matched(path, folder).is_ignore()))
    }

    pub fn run(&self, entries: Range<u32>, visit: &mut dyn FnMut(Visit<'_>) -> bool) {
        let index = self.index;
        let mut trail = Trail::new();
        let mut at = entries.start;
        for above in index.chain(at) {
            let record = index.record(above);
            trail.step(self.root, record.depth as usize, index.name(&record));
            if self.skipped(&record, trail.as_path()) {
                at = record.end.max(at);
                break;
            }
        }
        let mut failure = index.failures(at);
        while at < entries.end.min(index.entries) {
            let record = index.record(at);
            let depth = record.depth as usize;
            trail.step(self.root, depth, index.name(&record));
            let form = record.form();
            if self.skipped(&record, trail.as_path()) {
                at = match form {
                    Form::Dir => record.end.max(at + 1),
                    _ => at + 1,
                };
                continue;
            }
            let entry = Item {
                path: trail.as_path(),
                name: match depth {
                    0 => trail
                        .as_path()
                        .file_name()
                        .unwrap_or(trail.as_path().as_os_str()),
                    _ => OsStr::from_bytes(index.name(&record)),
                },
                depth,
                form,
                follow: false,
            };
            if !visit(Visit::Entry(entry)) {
                return;
            }
            let state = self.state.get(record.slot() as usize).copied().unwrap_or(0);
            while let Some((entry, text)) = index.failures.get(failure)
                && *entry <= at
            {
                let read = self.depth.is_none_or(|deepest| depth < deepest);
                if *entry == at && state & DIRTY == 0 && read {
                    let message = index.failure(text.clone(), self.root);
                    if !visit(Visit::Failure(&message)) {
                        return;
                    }
                }
                failure += 1;
            }
            at += 1;
        }
    }
}

pub struct Built {
    pub entries: usize,
    pub folders: usize,
}

struct Draft {
    records: Vec<Record>,
    folders: Vec<Folder>,
    names: Vec<u8>,
    inside: Vec<Watched>,
    failures: Vec<(u32, String)>,
}

fn refuse(root: &Path, print: &str, reason: &str) -> String {
    if let Some(marker) = place(root, print, "no") {
        let _ = fs::write(marker, reason);
    }
    reason.to_string()
}

fn gather(root: &Path, started: i64, mut builder: WalkBuilder) -> Result<Draft, String> {
    let mut draft = Draft {
        records: Vec::new(),
        folders: Vec::new(),
        names: Vec::new(),
        inside: Vec::new(),
        failures: Vec::new(),
    };
    let prefix = root.as_os_str().as_bytes().len();
    let mut open: Vec<u32> = Vec::new();
    let mut device = None;
    builder.sort_by_file_name(|left, right| left.cmp(right));
    for found in builder.build() {
        let entry = match found {
            Ok(entry) => entry,
            Err(error) => {
                let last = draft.records.len().saturating_sub(1) as u32;
                draft.failures.push((last, error.to_string()));
                continue;
            }
        };
        let Some(kind) = entry.file_type() else {
            continue;
        };
        let at = draft.records.len() as u32;
        let depth = entry.depth();
        while open.len() > depth {
            if let Some(closed) = open.pop() {
                draft.records[closed as usize].end = at;
            }
        }
        let name = match depth {
            0 => &[][..],
            _ => entry.file_name().as_bytes(),
        };
        let parent = open.last().copied().unwrap_or(0);
        if depth > usize::from(u16::MAX) || name.len() > usize::from(u16::MAX) || at >= 1 << 29 {
            return Err("the tree is too large or too deep for an index".to_string());
        }
        let form = Form::of(kind);
        let mut record = Record {
            name: draft.names.len() as u32,
            parent,
            end: at + 1,
            len: name.len() as u16,
            depth: depth as u16,
            tag: Record::tag(
                form,
                draft.records.get(parent as usize).map_or(0, Record::slot),
            ),
        };
        draft.names.extend(name);
        if form == Form::Dir {
            let slot = draft.folders.len() as u32;
            record.tag = Record::tag(form, slot);
            let path = entry.path();
            let stamp = fs::symlink_metadata(path)
                .map(|metadata| Stamp::of(&metadata))
                .unwrap_or_default();
            if *device.get_or_insert(stamp.device) != stamp.device {
                return Err(format!("{} is on another file system", path.display()));
            }
            let found = probe(path);
            let racy = if stamp.recent(started) || stamp == Stamp::default() {
                RACY
            } else {
                0
            };
            draft.folders.push(Folder {
                entry: at,
                flags: found | racy,
                stamp,
            });
            let rest = path.as_os_str().as_bytes().get(prefix..).unwrap_or(&[]);
            let start = rest.iter().take_while(|byte| **byte == b'/').count();
            let inner = Path::new(OsStr::from_bytes(&rest[start..]));
            for (name, bit) in [(".gitignore", GITIGNORE), (".ignore", IGNORE)] {
                if found & bit != 0 {
                    draft
                        .inside
                        .push(Watched::new(&path.join(name), &inner.join(name), BELOW));
                }
            }
            if found & GIT != 0 {
                for watched in git(path) {
                    match watched.strip_prefix(path) {
                        Ok(rest) => {
                            draft
                                .inside
                                .push(Watched::new(&watched, &inner.join(rest), BELOW))
                        }
                        Err(_) => draft.inside.push(Watched::new(&watched, &watched, 0)),
                    }
                }
            }
            open.push(at);
        }
        draft.records.push(record);
    }
    let total = draft.records.len() as u32;
    for closed in open {
        draft.records[closed as usize].end = total;
    }
    if draft.folders.is_empty() {
        return Err("the path is not a folder that can be read".to_string());
    }
    Ok(draft)
}

fn encode(draft: &Draft, real: &Path, print: &str, started: i64) -> Vec<u8> {
    let outside = outer(real);
    let mut context = Vec::new();
    for watched in outside.iter().chain(&draft.inside) {
        context.extend((watched.path.len() as u16).to_le_bytes());
        context.push(watched.mode);
        context.push(watched.kind);
        context.extend(&watched.path);
        watched.stamp.write(&mut context);
    }
    let mut errors = Vec::new();
    for (entry, text) in &draft.failures {
        let text = &text.as_bytes()[..text.len().min(usize::from(u16::MAX))];
        errors.extend(entry.to_le_bytes());
        errors.extend((text.len() as u16).to_le_bytes());
        errors.extend(text);
    }
    let root = real.as_os_str().as_bytes();
    let mut out = Vec::with_capacity(
        HEAD + draft.records.len() * RECORD + draft.folders.len() * FOLDER + draft.names.len(),
    );
    out.extend(MAGIC);
    out.extend(started.to_le_bytes());
    for count in [
        draft.records.len(),
        draft.folders.len(),
        draft.names.len(),
        root.len(),
        print.len(),
        context.len(),
        errors.len(),
        outside.len(),
    ] {
        out.extend((count as u32).to_le_bytes());
    }
    out.extend(root);
    out.extend(print.as_bytes());
    for record in &draft.records {
        record.write(&mut out);
    }
    for folder in &draft.folders {
        out.extend(folder.entry.to_le_bytes());
        out.extend(folder.flags.to_le_bytes());
        folder.stamp.write(&mut out);
    }
    out.extend(&draft.names);
    out.extend(context);
    out.extend(errors);
    out
}

fn prune(store: &Path) {
    let Ok(listing) = fs::read_dir(store) else {
        return;
    };
    let mut kept: Vec<(SystemTime, PathBuf)> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "idx"))
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    kept.sort();
    let extra = kept.len().saturating_sub(KEPT);
    for (_, path) in kept.into_iter().take(extra) {
        let _ = fs::remove_file(path);
    }
}

pub fn build(root: &Path, print: &str, builder: WalkBuilder) -> Result<Built, String> {
    let real = fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let store = store().ok_or("there is no cache folder, because HOME is not set")?;
    fs::create_dir_all(&store).map_err(|error| format!("{}: {error}", store.display()))?;
    if !trusted(&real) {
        return Err(refuse(
            &real,
            print,
            "an index works on local Linux file systems only",
        ));
    }
    let name = key(&real, print);
    let lock = store.join(format!("{name}.lock"));
    if younger(&lock, LOCKED) {
        return Err("another syk is building this index".to_string());
    }
    let _ = fs::remove_file(&lock);
    File::options()
        .write(true)
        .create_new(true)
        .open(&lock)
        .map_err(|error| format!("{}: {error}", lock.display()))?;
    let started = now();
    let made = gather(root, started, builder).and_then(|draft| {
        let bytes = encode(&draft, &real, print, started);
        let partial = store.join(format!("{name}.{}.part", std::process::id()));
        let written = File::create(&partial)
            .and_then(|mut file| file.write_all(&bytes))
            .and_then(|()| fs::rename(&partial, store.join(format!("{name}.idx"))));
        if let Err(error) = written {
            let _ = fs::remove_file(&partial);
            return Err(format!("{}: {error}", store.display()));
        }
        Ok(Built {
            entries: draft.records.len(),
            folders: draft.folders.len(),
        })
    });
    let _ = fs::remove_file(&lock);
    match made {
        Ok(built) => {
            let _ = fs::remove_file(store.join(format!("{name}.no")));
            prune(&store);
            Ok(built)
        }
        Err(reason) => Err(refuse(&real, print, &reason)),
    }
}

pub fn wanted(root: &Path, print: &str) -> bool {
    let Ok(real) = fs::canonicalize(root) else {
        return false;
    };
    let busy = place(&real, print, "lock").is_some_and(|lock| younger(&lock, LOCKED));
    let refused = place(&real, print, "no").is_some_and(|marker| younger(&marker, REFUSED));
    !busy && !refused && trusted(&real)
}

pub fn spawn(root: &Path, flags: &[String]) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    Command::new(std::env::current_exe()?)
        .args(flags)
        .arg("--index")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}
