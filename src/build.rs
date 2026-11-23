use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use ignore::WalkBuilder;

use crate::check::{git, outer, probe, trusted};
use crate::index::{
    BELOW, FOLDER, Folder, GIT, GITIGNORE, HEAD, IGNORE, Index, MAGIC, RACY, RECORD, Record, Stamp,
    Watched, key, now, place, store,
};
use crate::item::Form;
use crate::sign;
use crate::walk::Bars;

const KEPT: usize = 24;
const SIGNED: u64 = 1 << 30;
const LOCKED: Duration = Duration::from_secs(600);
const REFUSED: Duration = Duration::from_secs(24 * 60 * 60);
const RESTED: Duration = Duration::from_secs(60);

fn younger(path: &Path, limit: Duration) -> bool {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.elapsed().ok())
        .is_some_and(|age| age < limit)
}

pub struct Built {
    pub entries: usize,
    pub folders: usize,
    pub signed: usize,
}

struct Draft {
    records: Vec<Record>,
    folders: Vec<Folder>,
    names: Vec<u8>,
    inside: Vec<Watched>,
    failures: Vec<(u32, String)>,
}

impl Draft {
    fn record(
        &mut self,
        at: u32,
        name: &[u8],
        parent: u32,
        depth: usize,
        form: Form,
    ) -> Result<Record, String> {
        if depth > usize::from(u16::MAX) || name.len() > usize::from(u16::MAX) || at >= 1 << 29 {
            return Err("the tree is too large or too deep for an index".to_string());
        }
        let record = Record {
            name: self.names.len() as u32,
            parent,
            end: at + 1,
            len: name.len() as u16,
            depth: depth as u16,
            tag: Record::tag(
                form,
                self.records.get(parent as usize).map_or(0, Record::slot),
            ),
        };
        self.names.extend(name);
        self.names.push(b'\n');
        Ok(record)
    }

    fn folder(&mut self, at: u32, path: &Path, stamp: Stamp, started: i64) -> u32 {
        let found = probe(path);
        let racy = if stamp.recent(started) || stamp == Stamp::default() {
            RACY
        } else {
            0
        };
        self.folders.push(Folder {
            entry: at,
            flags: found | racy,
            stamp,
        });
        found
    }

    fn watch(&mut self, path: &Path, prefix: usize, found: u32) {
        let rest = path.as_os_str().as_bytes().get(prefix..).unwrap_or(&[]);
        let start = rest.iter().take_while(|byte| **byte == b'/').count();
        let inner = Path::new(OsStr::from_bytes(&rest[start..]));
        for (name, bit) in [(".gitignore", GITIGNORE), (".ignore", IGNORE)] {
            if found & bit != 0 {
                self.inside
                    .push(Watched::new(&path.join(name), &inner.join(name), BELOW));
            }
        }
        if found & GIT != 0 {
            for watched in git(path) {
                match watched.strip_prefix(path) {
                    Ok(rest) => self
                        .inside
                        .push(Watched::new(&watched, &inner.join(rest), BELOW)),
                    Err(_) => self.inside.push(Watched::new(&watched, &watched, 0)),
                }
            }
        }
    }
}

fn refuse(root: &Path, print: &str, reason: &str) -> String {
    if let Some(marker) = place(root, print, "no") {
        let _ = fs::write(marker, reason);
    }
    reason.to_string()
}

fn stamp(path: &Path, depth: usize) -> Stamp {
    let found = match depth {
        0 => fs::metadata(path),
        _ => fs::symlink_metadata(path),
    };
    found
        .map(|metadata| Stamp::of(&metadata))
        .unwrap_or_default()
}

struct Reach {
    parent: PathBuf,
    open: bool,
    devices: HashMap<u64, bool>,
}

fn bound(builder: &mut WalkBuilder, root: &Path, bars: Bars) {
    let first = fs::metadata(root).map_or(0, |found| found.dev());
    let reach = Mutex::new(Reach {
        parent: PathBuf::new(),
        open: true,
        devices: HashMap::from([(first, true)]),
    });
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        if bars.bar(entry) {
            return false;
        }
        let (Some(parent), Ok(mut reach)) = (entry.path().parent(), reach.lock()) else {
            return true;
        };
        if entry.depth() > 1 && reach.parent != parent {
            let device = fs::symlink_metadata(parent).map(|found| found.dev());
            reach.open = device.is_ok_and(|device| {
                *reach
                    .devices
                    .entry(device)
                    .or_insert_with(|| trusted(parent))
            });
            reach.parent = parent.to_path_buf();
        }
        entry.depth() == 1 || reach.open
    });
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
    let mut devices: HashMap<u64, bool> = HashMap::new();
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
        let form = Form::of(kind);
        let mut record = draft.record(at, name, parent, depth, form)?;
        if form == Form::Dir {
            let slot = draft.folders.len() as u32;
            record.tag = Record::tag(form, slot);
            let path = entry.path();
            let stamp = stamp(path, depth);
            let stored = *devices
                .entry(stamp.device)
                .or_insert_with(|| depth == 0 || trusted(path));
            let found = draft.folder(at, path, stamp, started);
            match (stored, draft.folders.last_mut()) {
                (false, Some(folder)) => folder.flags |= RACY,
                _ => draft.watch(path, prefix, found),
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

pub fn prune(store: &Path) {
    let Ok(listing) = fs::read_dir(store) else {
        return;
    };
    let paths: Vec<PathBuf> = listing.flatten().map(|entry| entry.path()).collect();
    let kind = |path: &Path, wanted: &str| path.extension().is_some_and(|kind| kind == wanted);
    for path in &paths {
        let lapsed = (kind(path, "no") && !younger(path, REFUSED))
            || ((kind(path, "lock") || kind(path, "part")) && !younger(path, LOCKED))
            || (kind(path, "sig") && !path.with_extension("idx").exists());
        if lapsed {
            let _ = fs::remove_file(path);
        }
    }
    let mut kept: Vec<(SystemTime, PathBuf)> = paths
        .into_iter()
        .filter(|path| kind(path, "idx"))
        .filter_map(|path| Some((fs::metadata(&path).ok()?.modified().ok()?, path)))
        .collect();
    kept.sort();
    let mut held = 0;
    for (at, (_, path)) in kept.into_iter().rev().enumerate() {
        let signs = path.with_extension("sig");
        held += fs::metadata(&signs).map_or(0, |found| found.len());
        if at >= KEPT || held > SIGNED {
            let _ = fs::remove_file(signs);
        }
        if at >= KEPT {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn build(
    root: &Path,
    print: &str,
    mut builder: WalkBuilder,
    bars: Bars,
    threads: usize,
) -> Result<Built, String> {
    bound(&mut builder, root, bars);
    let real = fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let store = store().ok_or("there is no cache folder, because HOME is not set")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&store)
        .map_err(|error| format!("{}: {error}", store.display()))?;
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
    let placed = store.join(format!("{name}.idx"));
    let partial = store.join(format!("{name}.{}.part", std::process::id()));
    let made = gather(root, started, builder).and_then(|draft| {
        let bytes = encode(&draft, &real, print, started);
        let written = File::options()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&partial)
            .and_then(|mut file| file.write_all(&bytes))
            .and_then(|()| fs::rename(&partial, &placed));
        if let Err(error) = written {
            let _ = fs::remove_file(&partial);
            return Err(format!("{}: {error}", store.display()));
        }
        let _ = fs::remove_file(placed.with_extension("sig"));
        let signed = Index::load(&placed).and_then(|index| {
            let signed = sign::build(&index, root.as_os_str().as_bytes(), threads, &partial)
                .and_then(|signed| {
                    fs::rename(&partial, placed.with_extension("sig")).map(|()| signed)
                });
            if signed.is_err() {
                let _ = fs::remove_file(&partial);
            }
            signed.ok()
        });
        Ok(Built {
            entries: draft.records.len(),
            folders: draft.folders.len(),
            signed: signed.unwrap_or(0),
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
    let rested = !place(&real, print, "idx").is_some_and(|index| younger(&index, RESTED));
    !busy && !refused && rested && trusted(&real)
}

pub fn spawn(root: &Path, flags: &[String]) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    let mut command = Command::new(std::env::current_exe()?);
    command
        .args(flags)
        .arg("--index")
        .arg(root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // The child lowers its own priority before it becomes syk, and touches nothing else
    unsafe {
        command.pre_exec(|| {
            libc::setpriority(libc::PRIO_PROCESS, 0, 19);
            Ok(())
        });
    }
    command.spawn().map(drop)
}
