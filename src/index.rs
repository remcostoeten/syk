use std::ffi::OsStr;
use std::fs::{self, File, FileTimes, Metadata};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use memmap2::Mmap;

use crate::item::Form;
use crate::sign::Signs;

pub const MAGIC: &[u8; 8] = b"sykidx03";
pub const HEAD: usize = 48;
pub const RECORD: usize = 20;
pub const FOLDER: usize = 40;
const STAMP: usize = 32;
pub const SLACK: i64 = 2_000_000_000;
const HOUR: Duration = Duration::from_secs(3600);
pub const RACY: u32 = 1;
pub const GITIGNORE: u32 = 2;
pub const IGNORE: u32 = 4;
pub const GIT: u32 = 8;
pub const JJ: u32 = 16;
pub const BELOW: u8 = 1;
pub const BARE: u8 = 2;
pub const STRIDE: u32 = 512;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Stamp {
    modified: i64,
    changed: i64,
    inode: u64,
    pub device: u64,
}

impl Stamp {
    pub fn of(metadata: &Metadata) -> Stamp {
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

    pub fn recent(&self, built: i64) -> bool {
        self.modified >= built - SLACK || self.changed >= built - SLACK
    }

    pub fn write(&self, out: &mut Vec<u8>) {
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

pub fn word(bytes: &[u8], at: usize) -> u32 {
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

pub fn long(bytes: &[u8], at: usize) -> u64 {
    bytes
        .get(at..at + 8)
        .and_then(|part| part.try_into().ok())
        .map_or(0, u64::from_le_bytes)
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos() as i64)
}

#[derive(Clone, Copy, Default)]
pub struct Record {
    pub name: u32,
    pub parent: u32,
    pub end: u32,
    pub len: u16,
    pub depth: u16,
    pub tag: u32,
}

impl Record {
    pub fn form(&self) -> Form {
        match self.tag & 3 {
            0 => Form::File,
            1 => Form::Dir,
            2 => Form::Link,
            _ => Form::Other,
        }
    }

    pub fn slot(&self) -> u32 {
        self.tag >> 2
    }

    pub fn tag(form: Form, slot: u32) -> u32 {
        let kind = match form {
            Form::File => 0,
            Form::Dir => 1,
            Form::Link => 2,
            Form::Other => 3,
        };
        slot << 2 | kind
    }

    pub fn write(&self, out: &mut Vec<u8>) {
        out.extend(self.name.to_le_bytes());
        out.extend(self.parent.to_le_bytes());
        out.extend(self.end.to_le_bytes());
        out.extend(self.len.to_le_bytes());
        out.extend(self.depth.to_le_bytes());
        out.extend(self.tag.to_le_bytes());
    }
}

#[derive(Clone, Copy, Default)]
pub struct Folder {
    pub entry: u32,
    pub flags: u32,
    pub stamp: Stamp,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Watched {
    pub path: Vec<u8>,
    pub mode: u8,
    pub kind: u8,
    pub stamp: Stamp,
}

impl Watched {
    pub fn see(path: &Path, mode: u8) -> (u8, Stamp) {
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

    pub fn new(path: &Path, stored: &Path, mode: u8) -> Watched {
        let (kind, stamp) = Watched::see(path, mode);
        Watched {
            path: stored.as_os_str().as_bytes().to_vec(),
            mode,
            kind,
            stamp,
        }
    }
}

pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

pub fn store() -> Option<PathBuf> {
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|folder| folder.is_absolute())
        .or_else(|| home().map(|home| home.join(".cache")))?;
    Some(cache.join("syk").join("index"))
}

pub fn key(root: &Path, print: &str) -> String {
    // FNV-1a, so the file name stays the same between versions of the compiler
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let parts = [root.as_os_str().as_bytes(), &[0], print.as_bytes()];
    for byte in parts.into_iter().flatten() {
        hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

pub fn place(root: &Path, print: &str, kind: &str) -> Option<PathBuf> {
    Some(store()?.join(format!("{}.{kind}", key(root, print))))
}

pub struct Trail {
    pub path: Vec<u8>,
    lens: Vec<usize>,
}

impl Trail {
    pub fn new() -> Trail {
        Trail {
            path: Vec::with_capacity(256),
            lens: Vec::with_capacity(32),
        }
    }

    pub fn step(&mut self, root: &[u8], depth: usize, name: &[u8]) {
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

    pub fn as_path(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.path))
    }
}

pub struct Index {
    pub map: Mmap,
    path: PathBuf,
    pub built: i64,
    pub entries: u32,
    pub folders: u32,
    pub outside: u32,
    pub root: Range<usize>,
    pub print: Range<usize>,
    records: usize,
    slots: usize,
    pub names: Range<usize>,
    context: Range<usize>,
    pub failures: Vec<(u32, Range<usize>)>,
    pub span: Range<u32>,
    pub within: Range<u32>,
    pub floor: u16,
    pub inner: Vec<u8>,
}

impl Index {
    pub fn load(path: &Path) -> Option<Index> {
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
            path: path.to_path_buf(),
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
            span: 0..entries,
            within: 0..folders,
            floor: 0,
            inner: Vec::new(),
        })
    }

    pub fn entries(&self) -> u32 {
        self.entries
    }

    pub fn folders(&self) -> u32 {
        self.folders
    }

    pub fn root(&self) -> &Path {
        Path::new(OsStr::from_bytes(&self.map[self.root.clone()]))
    }

    pub fn used(self) -> Index {
        let lately = fs::metadata(&self.path)
            .and_then(|found| found.accessed())
            .is_ok_and(|used| used.elapsed().is_ok_and(|age| age < HOUR));
        if !lately && let Ok(file) = File::open(&self.path) {
            let _ = file.set_times(FileTimes::new().set_accessed(SystemTime::now()));
        }
        self
    }

    pub fn view(&self) -> &'static str {
        let print = &self.map[self.print.clone()];
        match print.split(|byte| *byte == b'\n').next() {
            Some(b"1H") => "-H",
            Some(b"1I") => "-I",
            Some(b"1HI") => "-H -I",
            Some(b"1A") => "-a",
            _ => "",
        }
    }

    pub fn built(&self) -> i64 {
        self.built
    }

    pub fn signs(&self) -> Option<Signs> {
        Signs::open(&self.path.with_extension("sig"), self.built, self.entries)
    }

    pub fn files(&self, root: &[u8], entries: Range<u32>, visit: &mut dyn FnMut(u32, &Path)) {
        let entries = entries.start..entries.end.min(self.entries);
        if entries.is_empty() {
            return;
        }
        let mut trail = Trail::new();
        for above in self.chain(entries.start) {
            let record = self.record(above);
            trail.step(root, record.depth as usize, self.name(&record));
        }
        for at in entries {
            let record = self.record(at);
            trail.step(root, record.depth as usize, self.name(&record));
            if record.form() == Form::File {
                visit(at, trail.as_path());
            }
        }
    }

    pub fn record(&self, at: u32) -> Record {
        let start = self.records + at as usize * RECORD;
        match self.map.get(start..start + RECORD) {
            Some(bytes) if at < self.entries => Record {
                name: word(bytes, 0),
                parent: word(bytes, 4),
                end: word(bytes, 8),
                len: short(bytes, 12),
                depth: short(bytes, 14).saturating_sub(self.floor),
                tag: word(bytes, 16),
            },
            _ => Record::default(),
        }
    }

    pub fn name(&self, record: &Record) -> &[u8] {
        let start = self.names.start + record.name as usize;
        self.map
            .get(start..start + record.len as usize)
            .filter(|name| start + name.len() <= self.names.end)
            .unwrap_or(&[])
    }

    pub fn folder(&self, slot: u32) -> Folder {
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

    pub fn watched(&self) -> Vec<Watched> {
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

    pub fn chain(&self, at: u32) -> Vec<u32> {
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

    pub fn failure(&self, text: Range<usize>, root: &[u8]) -> String {
        let stored = String::from_utf8_lossy(&self.map[text]);
        let real = match self.inner.is_empty() {
            true => &self.map[self.root.clone()],
            false => &self.inner[..],
        };
        let real = String::from_utf8_lossy(real);
        let given = String::from_utf8_lossy(root);
        let inside = format!("{}/", real.trim_end_matches('/'));
        match stored.strip_prefix(real.as_ref()) {
            Some(rest) if !stored.starts_with(&inside) => format!("{given}{rest}"),
            _ => stored.replace(&inside, &format!("{}/", given.trim_end_matches('/'))),
        }
    }

    pub fn failures(&self, from: u32) -> usize {
        self.failures.partition_point(|(entry, _)| *entry < from)
    }
}
