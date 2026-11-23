use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::index::{
    BARE, BELOW, GIT, GITIGNORE, IGNORE, Index, JJ, RACY, STRIDE, Stamp, Trail, Watched, home,
    place,
};
use crate::item::Form;

const PROBES: u32 = GITIGNORE | IGNORE | GIT | JJ;

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

pub fn probe(dir: &Path) -> u32 {
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

pub fn git(dir: &Path) -> Vec<PathBuf> {
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

pub fn outer(root: &Path) -> Vec<Watched> {
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
    if let Some(excludes) = crate::walk::excludes_file() {
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
pub fn trusted(path: &Path) -> bool {
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
pub fn trusted(_: &Path) -> bool {
    false
}

pub enum Opened {
    Ready(Index),
    Stale,
    Missing,
}

pub struct Fresh {
    pub dirty: HashMap<Vec<u8>, u32>,
    pub above: HashSet<Vec<u8>>,
    pub kids: HashMap<Vec<u8>, u32>,
}

impl Index {
    pub fn open(root: &Path, print: &str) -> Opened {
        let folder = fs::metadata(root).is_ok_and(|found| found.is_dir());
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
            let found = match record.depth {
                0 => fs::metadata(trail.as_path()),
                _ => fs::symlink_metadata(trail.as_path()),
            };
            let clean = folder.flags & RACY == 0
                && found.is_ok_and(|found| found.is_dir() && Stamp::of(&found) == folder.stamp);
            if !clean {
                dirty.push(slot);
            }
        }
    }

    pub fn stale(&self, root: &[u8], threads: usize) -> Vec<u32> {
        let next = AtomicU32::new(0);
        let crew = threads.clamp(1, self.folders.div_ceil(STRIDE) as usize);
        let mut dirty: Vec<u32> = std::thread::scope(|scope| {
            let crew: Vec<_> = (0..crew)
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
}
