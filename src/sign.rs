use std::fs::{self, File, Metadata};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use memmap2::Mmap;

use crate::index::{Index, SLACK, long, word};
use crate::load::{self, MARKS, WHOLE};
use crate::need::Need;

const MAGIC: &[u8; 8] = b"syksig02";
const HEAD: usize = 128;
const MARK: usize = 48;
const CLASSES: usize = 18;
const SMALLEST: usize = 64;
const LARGEST: u64 = 64 << 20;
const ROOM: usize = 256 << 20;
const BINARY: u8 = 0xFF;
const STRIDE: u32 = 256;
const FIRST: usize = 4096;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Seal {
    modified: i64,
    changed: i64,
    inode: u64,
    size: u64,
}

impl Seal {
    fn of(metadata: &Metadata) -> Option<Seal> {
        let nanos = |seconds: i64, rest: i64| seconds.saturating_mul(1_000_000_000) + rest;
        metadata.is_file().then(|| Seal {
            modified: nanos(metadata.mtime(), metadata.mtime_nsec()),
            changed: nanos(metadata.ctime(), metadata.ctime_nsec()),
            inode: metadata.ino(),
            size: metadata.len(),
        })
    }

    fn read(path: &Path) -> Option<Seal> {
        Seal::of(&fs::symlink_metadata(path).ok()?)
    }

    fn recent(&self, built: i64) -> bool {
        self.modified >= built - SLACK || self.changed >= built - SLACK
    }
}

#[derive(Clone, Copy)]
pub struct Ruled {
    seal: Seal,
    pub binary: bool,
}

impl Ruled {
    pub fn holds(&self, path: &Path) -> bool {
        Seal::read(path) == Some(self.seal)
    }
}

// Spreads the three bytes of a gram over all 32 bits
fn hash(gram: u32) -> u32 {
    let mut mixed = gram.wrapping_mul(0x9E37_79B1);
    mixed ^= mixed >> 15;
    mixed = mixed.wrapping_mul(0x85EB_CA6B);
    mixed ^ mixed >> 13
}

fn width(class: usize) -> usize {
    SMALLEST << class
}

enum Signed {
    Text(u8, Vec<u8>),
    Binary,
    Unknown,
}

struct Signer {
    bits: Vec<u64>,
    chunk: Vec<u8>,
    gram: u32,
    run: u32,
}

impl Signer {
    fn new() -> Signer {
        Signer {
            bits: Vec::new(),
            chunk: vec![0; WHOLE],
            gram: 0,
            run: 0,
        }
    }

    fn fill(&mut self, file: &mut File, mut length: usize) -> io::Result<usize> {
        while length < self.chunk.len() {
            match file.read(&mut self.chunk[length..]) {
                Ok(0) => break,
                Ok(read) => length += read,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(length)
    }

    fn feed(&mut self, length: usize) {
        let mask = (self.bits.len() * 64 - 1) as u32;
        let (mut gram, mut run) = (self.gram, self.run);
        for byte in &self.chunk[..length] {
            gram = (gram << 8 | u32::from(byte.to_ascii_lowercase())) & 0xFF_FFFF;
            run = if *byte == b'\n' { 0 } else { (run + 1).min(3) };
            if run == 3 {
                let at = hash(gram) & mask;
                self.bits[(at >> 6) as usize] |= 1 << (at & 63);
            }
        }
        (self.gram, self.run) = (gram, run);
    }

    fn fold(&mut self) -> Option<u8> {
        let bits = &mut self.bits;
        let mut words = bits.len();
        while words > 1 {
            let half = words / 2;
            let ones: u64 = (0..half)
                .map(|at| u64::from((bits[at] | bits[at + half]).count_ones()))
                .sum();
            if ones * 10 > half as u64 * 64 * 3 {
                break;
            }
            for at in 0..half {
                bits[at] |= bits[at + half];
            }
            words = half;
        }
        bits.truncate(words);
        let ones: u64 = bits.iter().map(|word| u64::from(word.count_ones())).sum();
        (ones * 2 <= words as u64 * 64).then(|| words.trailing_zeros() as u8)
    }

    fn sign(&mut self, path: &Path, size: u64) -> Signed {
        let Ok(mut file) = File::open(path) else {
            return Signed::Unknown;
        };
        let Ok((length, warm)) = load::peek(&mut file, &mut self.chunk[..FIRST]) else {
            return Signed::Unknown;
        };
        let signed = self.read(&mut file, length, size);
        if !warm {
            load::forget(&file);
        }
        signed
    }

    fn read(&mut self, file: &mut File, first: usize, size: u64) -> Signed {
        let head = &self.chunk[..first];
        if MARKS.iter().any(|mark| head.starts_with(mark)) {
            return Signed::Unknown;
        }
        if memchr::memchr(0, head).is_some() {
            return Signed::Binary;
        }
        let Ok(mut length) = self.fill(file, first) else {
            return Signed::Unknown;
        };
        if memchr::memchr(0, &self.chunk[..length]).is_some() {
            return Signed::Binary;
        }
        if size > LARGEST {
            return Signed::Unknown;
        }
        let wanted = usize::try_from(size.max(length as u64))
            .unwrap_or(usize::MAX)
            .saturating_mul(2)
            .checked_next_power_of_two()
            .unwrap_or(usize::MAX)
            .clamp(SMALLEST, width(CLASSES - 1));
        self.bits.clear();
        self.bits.resize(wanted / 64, 0);
        (self.gram, self.run) = (0, 0);
        while length > 0 {
            self.feed(length);
            length = match self.fill(file, 0) {
                Ok(length) => length,
                Err(_) => return Signed::Unknown,
            };
        }
        match self.fold() {
            Some(class) => {
                let bytes = self.bits.iter().flat_map(|word| word.to_le_bytes());
                Signed::Text(class, bytes.collect())
            }
            None => Signed::Unknown,
        }
    }
}

struct Mark {
    entry: u32,
    column: u32,
    class: u8,
    seal: Seal,
}

#[derive(Default)]
struct Draft {
    marks: Vec<Mark>,
    major: [Vec<u8>; CLASSES],
    held: usize,
}

impl Draft {
    fn add(&mut self, entry: u32, seal: Seal, signed: Signed) {
        let (class, column) = match signed {
            Signed::Unknown => return,
            Signed::Binary => (BINARY, 0),
            Signed::Text(class, bytes) => {
                let major = &mut self.major[usize::from(class)];
                let column = major.len() / (width(usize::from(class)) / 8);
                self.held += bytes.len();
                major.extend(bytes);
                (class, column as u32)
            }
        };
        self.marks.push(Mark {
            entry,
            column,
            class,
            seal,
        });
    }

    fn turned(&self, class: usize) -> Vec<u8> {
        let bytes = width(class) / 8;
        let columns = self.major[class].len() / bytes;
        let stride = columns.div_ceil(8);
        let mut rows = vec![0u8; width(class) * stride];
        for (column, sign) in self.major[class].chunks_exact(bytes).enumerate() {
            for (at, byte) in sign.iter().enumerate().filter(|(_, byte)| **byte != 0) {
                for bit in (0..8).filter(|bit| byte >> bit & 1 == 1) {
                    rows[(at * 8 + bit) * stride + column / 8] |= 1 << (column % 8);
                }
            }
        }
        rows
    }

    fn write(mut self, path: &Path, built: i64, entries: u32) -> io::Result<usize> {
        self.marks.sort_unstable_by_key(|mark| mark.entry);
        let mut head = Vec::with_capacity(HEAD + self.marks.len() * MARK);
        head.extend(MAGIC);
        head.extend(built.to_le_bytes());
        head.extend(entries.to_le_bytes());
        head.extend((self.marks.len() as u32).to_le_bytes());
        head.extend([0u8; 8]);
        for class in 0..CLASSES {
            let columns = self.major[class].len() / (width(class) / 8);
            head.extend((columns as u32).to_le_bytes());
        }
        head.resize(HEAD, 0);
        for mark in &self.marks {
            head.extend(mark.entry.to_le_bytes());
            head.extend(mark.column.to_le_bytes());
            head.extend([mark.class, 0, 0, 0, 0, 0, 0, 0]);
            head.extend(mark.seal.modified.to_le_bytes());
            head.extend(mark.seal.changed.to_le_bytes());
            head.extend(mark.seal.inode.to_le_bytes());
            head.extend(mark.seal.size.to_le_bytes());
        }
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(&head)?;
        for class in 0..CLASSES {
            file.write_all(&self.turned(class))?;
            self.major[class] = Vec::new();
        }
        Ok(self.marks.len())
    }
}

pub fn build(index: &Index, root: &[u8], threads: usize, partial: &Path) -> io::Result<usize> {
    let next = AtomicU32::new(0);
    let full = AtomicBool::new(false);
    let draft = Mutex::new(Draft::default());
    std::thread::scope(|scope| {
        for _ in 0..threads.max(1) {
            scope.spawn(|| {
                let mut signer = Signer::new();
                while !full.load(Ordering::Relaxed) {
                    let from = next.fetch_add(STRIDE, Ordering::Relaxed);
                    if from >= index.entries() {
                        break;
                    }
                    let mut found = Vec::new();
                    index.files(
                        root,
                        from..from.saturating_add(STRIDE),
                        &mut |entry, path| {
                            let settled = |seal: &Seal| !seal.recent(index.built());
                            if let Some(seal) = Seal::read(path).filter(settled) {
                                found.push((entry, seal, signer.sign(path, seal.size)));
                            }
                        },
                    );
                    let Ok(mut draft) = draft.lock() else {
                        break;
                    };
                    for (entry, seal, signed) in found {
                        draft.add(entry, seal, signed);
                    }
                    if draft.held > ROOM {
                        full.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
    });
    let draft = draft.into_inner().unwrap_or_default();
    draft.write(partial, index.built(), index.entries())
}

pub struct Signs {
    map: Mmap,
    marks: usize,
    rows: [(usize, usize); CLASSES],
}

impl Signs {
    pub fn open(path: &Path, built: i64, entries: u32) -> Option<Signs> {
        let file = File::open(path).ok()?;
        // The file is replaced by a rename and never changed in place
        let map = unsafe { Mmap::map(&file) }.ok()?;
        let same = map.len() >= HEAD
            && &map[..8] == MAGIC
            && long(&map, 8) as i64 == built
            && word(&map, 16) == entries;
        if !same {
            return None;
        }
        let marks = word(&map, 20) as usize;
        let mut at = HEAD.checked_add(marks.checked_mul(MARK)?)?;
        let mut rows = [(0, 0); CLASSES];
        for (class, row) in rows.iter_mut().enumerate() {
            let columns = word(&map, 32 + class * 4) as usize;
            *row = (columns, at);
            at = at.checked_add(width(class).checked_mul(columns.div_ceil(8))?)?;
        }
        (at == map.len()).then_some(Signs { map, marks, rows })
    }

    fn mark(&self, at: usize) -> Option<Mark> {
        let start = HEAD + at.checked_mul(MARK)?;
        let bytes = self.map.get(start..start + MARK)?;
        (at < self.marks).then(|| Mark {
            entry: word(bytes, 0),
            column: word(bytes, 4),
            class: bytes[8],
            seal: Seal {
                modified: long(bytes, 16) as i64,
                changed: long(bytes, 24) as i64,
                inode: long(bytes, 32),
                size: long(bytes, 40),
            },
        })
    }

    fn seek(&self, entry: u32) -> usize {
        let (mut low, mut high) = (0, self.marks);
        while low < high {
            let middle = low + (high - low) / 2;
            match self.mark(middle).is_some_and(|mark| mark.entry < entry) {
                true => low = middle + 1,
                false => high = middle,
            }
        }
        low
    }

    fn row(&self, class: usize, at: usize) -> &[u8] {
        let (columns, offset) = self.rows[class];
        let stride = columns.div_ceil(8);
        let start = offset + at * stride;
        self.map.get(start..start + stride).unwrap_or(&[])
    }

    fn weigh(&self, need: &Need, class: usize) -> Vec<u8> {
        let stride = self.rows[class].0.div_ceil(8);
        let merge = |needs: &[Need], start: u8, join: fn(u8, u8) -> u8| {
            needs.iter().fold(vec![start; stride], |mut kept, need| {
                for (byte, other) in kept.iter_mut().zip(self.weigh(need, class)) {
                    *byte = join(*byte, other);
                }
                kept
            })
        };
        match need {
            Need::Any => vec![0xFF; stride],
            Need::Gram(gram) => self
                .row(class, hash(*gram) as usize & (width(class) - 1))
                .to_vec(),
            Need::All(needs) => merge(needs, 0xFF, |left, right| left & right),
            Need::One(needs) => merge(needs, 0, |left, right| left | right),
        }
    }

    pub fn judge(&self, need: &Need) -> Verdicts<'_> {
        Verdicts {
            signs: self,
            open: (0..CLASSES).map(|class| self.weigh(need, class)).collect(),
        }
    }
}

pub struct Verdicts<'a> {
    signs: &'a Signs,
    open: Vec<Vec<u8>>,
}

impl Verdicts<'_> {
    pub fn ruling(&self, entry: u32, hint: &mut usize) -> Option<Ruled> {
        let signs = self.signs;
        if signs.mark(*hint).is_none_or(|mark| mark.entry != entry) {
            *hint = signs.seek(entry);
        }
        let mark = signs.mark(*hint).filter(|mark| mark.entry == entry)?;
        *hint += 1;
        if mark.class == BINARY {
            return Some(Ruled {
                seal: mark.seal,
                binary: true,
            });
        }
        let column = mark.column as usize;
        let open = self
            .open
            .get(usize::from(mark.class))
            .and_then(|open| open.get(column / 8))
            .is_none_or(|byte| byte >> (column % 8) & 1 == 1);
        (!open).then_some(Ruled {
            seal: mark.seal,
            binary: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::need;

    fn scratch(label: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("syk-sign-{}-{label}", std::process::id()));
        fs::create_dir_all(&folder).expect("create a folder");
        folder
    }

    fn signed(folder: &Path, name: &str, text: &[u8]) -> Signed {
        let path = folder.join(name);
        fs::write(&path, text).expect("write a file");
        Signer::new().sign(&path, text.len() as u64)
    }

    #[test]
    fn a_file_is_ruled_out_only_when_a_piece_is_missing() {
        let folder = scratch("rule");
        let texts: [&[u8]; 4] = [
            b"fn main() {\n    let needle = 1;\n}\n",
            b"nothing to see here\nNEEDLE in capitals\n",
            b"the nee\ndle is split over two lines\n",
            b"",
        ];
        let mut draft = Draft::default();
        for (entry, text) in texts.iter().enumerate() {
            let signed = signed(&folder, "text", text);
            draft.add(entry as u32 * 2, Seal::default(), signed);
        }
        draft.add(9, Seal::default(), signed(&folder, "binary", b"needle\0"));
        draft.add(
            11,
            Seal::default(),
            signed(&folder, "wide", b"\xFF\xFEn\0e\0"),
        );
        let path = folder.join("signs");
        assert_eq!(draft.write(&path, 7, 12).expect("write the signs"), 5);
        assert!(Signs::open(&path, 8, 12).is_none() && Signs::open(&path, 7, 13).is_none());
        let signs = Signs::open(&path, 7, 12).expect("open the signs");
        let ruled = |source: &str| -> Vec<u32> {
            let verdicts = signs.judge(&need::of(source, false));
            let mut hint = 0;
            (0..12)
                .filter(|entry| verdicts.ruling(*entry, &mut hint).is_some())
                .collect()
        };
        assert_eq!(ruled("needle"), [4, 6, 9]);
        assert_eq!(ruled("let needle|capitals"), [4, 6, 9]);
        assert_eq!(ruled("split.*lines"), [0, 2, 6, 9]);
        assert_eq!(ruled("zzzzzz"), [0, 2, 4, 6, 9]);
        let verdicts = signs.judge(&need::of("needle", false));
        assert!(verdicts.ruling(9, &mut 0).is_some_and(|ruled| ruled.binary));
        assert!(
            verdicts
                .ruling(4, &mut 3)
                .is_some_and(|ruled| !ruled.binary)
        );
        fs::remove_dir_all(folder).expect("remove the folder");
    }

    #[test]
    fn a_large_file_gets_a_larger_sign_and_keeps_every_piece() {
        let folder = scratch("large");
        let mut text = Vec::new();
        for line in 0..20_000u32 {
            text.extend(
                format!("line {line} holds {:x}\n", line.wrapping_mul(2_654_435_761)).bytes(),
            );
        }
        text.extend(b"the only needle\n");
        let mut signer = Signer::new();
        let path = folder.join("large");
        fs::write(&path, &text).expect("write a file");
        let Signed::Text(class, sign) = signer.sign(&path, text.len() as u64) else {
            panic!("a text file gets a sign");
        };
        assert!(class > 6 && sign.len() == width(usize::from(class)) / 8);
        let ones: u32 = sign.iter().map(|byte| byte.count_ones()).sum();
        assert!(ones as usize * 10 <= sign.len() * 8 * 3);
        let lowered = text.to_ascii_lowercase();
        for found in lowered.windows(3).filter_map(need::gram) {
            let at = hash(found) as usize & (sign.len() * 8 - 1);
            assert!(sign[at / 8] >> (at % 8) & 1 == 1, "a piece is missing");
        }
        fs::remove_dir_all(folder).expect("remove the folder");
    }

    #[test]
    fn a_seal_notices_every_change_to_a_file() {
        let folder = scratch("seal");
        let path = folder.join("file");
        fs::write(&path, b"first").expect("write a file");
        let seal = Seal::read(&path).expect("a seal");
        let ruled = Ruled {
            seal,
            binary: false,
        };
        assert!(ruled.holds(&path));
        assert!(seal.recent(crate::index::now()) && !seal.recent(i64::MAX));
        fs::write(&path, b"other").expect("write a file");
        let touched = Seal::read(&path).expect("a seal");
        assert!(touched.changed >= seal.changed && touched.size == seal.size);
        fs::write(&path, b"longer text").expect("write a file");
        assert!(!ruled.holds(&path));
        fs::remove_file(&path).expect("remove the file");
        assert!(!ruled.holds(&path) && Seal::read(&folder).is_none());
        fs::remove_dir_all(folder).expect("remove the folder");
    }
}
