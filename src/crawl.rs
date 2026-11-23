use std::ffi::OsStr;
use std::fs::{self, File};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::item::{Form, Item};
use crate::query::Query;
use crate::rule::{Base, Layer, Rules, Seen};
use crate::scan::Visit;

const PATIENCE: Duration = Duration::from_millis(2);
const ROOM: usize = 32 * 1024;
const HEAD: usize = 19;

pub trait Sink {
    fn take(&mut self, visit: Visit<'_>) -> bool;
    fn heavy(&self, name: &[u8], form: Form) -> bool;
}

struct Folder {
    path: Vec<u8>,
    depth: usize,
    root: usize,
    layer: Option<Arc<Layer>>,
}

#[derive(Default)]
struct Files {
    paths: Vec<u8>,
    marks: Vec<(usize, usize, Form)>,
    depth: usize,
}

enum Task {
    Folder(Folder),
    Files(Files),
}

pub struct Pace {
    pub few: u64,
    pub batch: usize,
}

struct Pool {
    rules: Rules,
    bases: Vec<Base>,
    depth: Option<usize>,
    batch: usize,
    queue: Mutex<Vec<Task>>,
    wake: Condvar,
    open: AtomicUsize,
    idle: AtomicUsize,
    quit: AtomicBool,
}

impl Pool {
    fn stop(&self) {
        self.quit.store(true, Ordering::SeqCst);
        drop(self.queue.lock());
        self.wake.notify_all();
    }
}

fn entries(listing: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut at = 0;
    std::iter::from_fn(move || {
        let head = listing.get(at..at + HEAD)?;
        let size = usize::from(u16::from_ne_bytes([head[16], head[17]])).max(HEAD + 1);
        let tail = listing.get(at + HEAD..at + size)?;
        at += size;
        let name = &tail[..memchr::memchr(0, tail).unwrap_or(tail.len())];
        Some((head[18], name))
    })
    .filter(|(_, name)| !matches!(*name, b"." | b".."))
}

fn form(kind: u8, path: &Path) -> io::Result<Form> {
    Ok(match kind {
        libc::DT_DIR => Form::Dir,
        libc::DT_REG => Form::File,
        libc::DT_LNK => Form::Link,
        libc::DT_UNKNOWN => Form::of(fs::symlink_metadata(path)?.file_type()),
        _ => Form::Other,
    })
}

struct Crew<'a, S> {
    pool: &'a Pool,
    sink: S,
    local: Vec<Task>,
    chunk: Vec<u8>,
    listing: Vec<u8>,
    path: Vec<u8>,
    seen: u64,
}

impl<'a, S: Sink> Crew<'a, S> {
    fn new(pool: &'a Pool, sink: S) -> Crew<'a, S> {
        Crew {
            pool,
            sink,
            local: Vec::new(),
            chunk: vec![0; ROOM],
            listing: Vec::new(),
            path: Vec::new(),
            seen: 0,
        }
    }

    fn next(&mut self) -> Option<Task> {
        let pool = self.pool;
        if pool.quit.load(Ordering::Relaxed) {
            return None;
        }
        if let Some(task) = self.local.pop() {
            return Some(task);
        }
        let mut queue = pool.queue.lock().ok()?;
        loop {
            if pool.quit.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(task) = queue.pop() {
                return Some(task);
            }
            if pool.open.load(Ordering::SeqCst) == 0 {
                return None;
            }
            pool.idle.fetch_add(1, Ordering::SeqCst);
            queue = pool.wake.wait(queue).ok()?;
            pool.idle.fetch_sub(1, Ordering::SeqCst);
        }
    }

    fn share(&mut self) {
        let pool = self.pool;
        if self.local.len() < 2 || pool.idle.load(Ordering::Relaxed) == 0 {
            return;
        }
        let given = self.local.len() / 2;
        if let Ok(mut queue) = pool.queue.lock() {
            queue.extend(self.local.drain(..given));
        }
        pool.wake.notify_all();
    }

    fn finish(&self) {
        let pool = self.pool;
        if pool.open.fetch_sub(1, Ordering::SeqCst) == 1 {
            drop(pool.queue.lock());
            pool.wake.notify_all();
        }
    }

    fn read(&mut self, path: &Path) -> io::Result<Option<io::Error>> {
        let folder = File::options()
            .read(true)
            .custom_flags(libc::O_DIRECTORY)
            .open(path)?;
        self.listing.clear();
        loop {
            let (start, room) = (self.chunk.as_mut_ptr(), self.chunk.len());
            // getdents64 fills at most `room` bytes of the chunk with whole entries
            let read =
                unsafe { libc::syscall(libc::SYS_getdents64, folder.as_raw_fd(), start, room) };
            match usize::try_from(read) {
                Ok(0) => return Ok(None),
                Ok(read) => self.listing.extend(&self.chunk[..read.min(room)]),
                Err(_) => {
                    let error = io::Error::last_os_error();
                    if error.kind() != io::ErrorKind::Interrupted {
                        return Ok(Some(error));
                    }
                }
            }
        }
    }

    fn layer(&self, folder: &Folder, path: &Path) -> Option<Arc<Layer>> {
        let pool = self.pool;
        if !pool.rules.active() {
            return None;
        }
        let mut seen = Seen::default();
        for (_, name) in entries(&self.listing).filter(|(_, name)| name.starts_with(b".")) {
            seen.note(name);
        }
        if !seen.any() {
            return folder.layer.clone();
        }
        let above = pool.bases[folder.root].any_git;
        let parent = folder.layer.clone();
        let layer = Layer::read(path, Some(&seen), parent, above, &mut Vec::new());
        Some(Arc::new(layer))
    }

    fn list(&mut self, folder: Folder) -> bool {
        let pool = self.pool;
        let base = &pool.bases[folder.root];
        if folder.depth == 0
            && let Some(failure) = &base.failure
            && !self.sink.take(Visit::Failure(failure))
        {
            return false;
        }
        let path = Path::new(OsStr::from_bytes(&folder.path));
        let wanted = pool.depth.is_none_or(|deepest| folder.depth < deepest);
        let opened = wanted.then(|| self.read(path));
        let own = Item {
            path,
            name: path.file_name().unwrap_or(path.as_os_str()),
            depth: folder.depth,
            form: Form::Dir,
            follow: false,
            ruled: None,
        };
        if !self.sink.take(Visit::Entry(own)) {
            return false;
        }
        let broken = match opened {
            None => return true,
            Some(Ok(broken)) => broken,
            Some(Err(error)) => {
                let failure = format!("{}: {error}", path.display());
                return self.sink.take(Visit::Failure(&failure));
            }
        };
        let layer = self.layer(&folder, path);
        if !self.children(&folder, layer) {
            return false;
        }
        match broken {
            Some(error) => {
                let failure = format!("{}: {error}", path.display());
                self.sink.take(Visit::Failure(&failure))
            }
            None => true,
        }
    }

    fn children(&mut self, folder: &Folder, layer: Option<Arc<Layer>>) -> bool {
        let pool = self.pool;
        let base = &pool.bases[folder.root];
        self.path.clear();
        self.path.extend(&folder.path);
        if !self.path.ends_with(b"/") {
            self.path.push(b'/');
        }
        let stem = self.path.len();
        let mut files = Files::default();
        for (kind, name) in entries(&self.listing) {
            self.seen += 1;
            self.path.truncate(stem);
            self.path.extend(name);
            let whole = Path::new(OsStr::from_bytes(&self.path));
            let form = match form(kind, whole) {
                Ok(form) => form,
                Err(error) => {
                    let failure = format!("{}: {error}", whole.display());
                    if !self.sink.take(Visit::Failure(&failure)) {
                        return false;
                    }
                    continue;
                }
            };
            let folded = form == Form::Dir;
            if folded && folder.depth == 0 && base.made(name) {
                continue;
            }
            if pool
                .rules
                .skip(base, layer.as_deref(), &self.path, name, folded)
            {
                continue;
            }
            if folded {
                pool.open.fetch_add(1, Ordering::SeqCst);
                self.local.push(Task::Folder(Folder {
                    path: self.path.clone(),
                    depth: folder.depth + 1,
                    root: folder.root,
                    layer: layer.clone(),
                }));
                continue;
            }
            if pool.batch > 0 && self.sink.heavy(name, form) {
                files.paths.extend(&self.path);
                files.marks.push((files.paths.len(), name.len(), form));
                if files.marks.len() == pool.batch {
                    files.depth = folder.depth + 1;
                    pool.open.fetch_add(1, Ordering::SeqCst);
                    self.local.push(Task::Files(std::mem::take(&mut files)));
                }
                continue;
            }
            let entry = Item {
                path: whole,
                name: OsStr::from_bytes(name),
                depth: folder.depth + 1,
                form,
                follow: false,
                ruled: None,
            };
            if !self.sink.take(Visit::Entry(entry)) {
                return false;
            }
        }
        if !files.marks.is_empty() {
            files.depth = folder.depth + 1;
            pool.open.fetch_add(1, Ordering::SeqCst);
            self.local.push(Task::Files(files));
        }
        true
    }

    fn visit(&mut self, files: Files) -> bool {
        let mut start = 0;
        files.marks.iter().all(|(end, name, form)| {
            let path = &files.paths[start..*end];
            start = *end;
            self.sink.take(Visit::Entry(Item {
                path: Path::new(OsStr::from_bytes(path)),
                name: OsStr::from_bytes(&path[path.len() - name..]),
                depth: files.depth,
                form: *form,
                follow: false,
                ruled: None,
            }))
        })
    }

    fn step(&mut self) -> bool {
        let done = match self.next() {
            Some(Task::Folder(folder)) => self.list(folder),
            Some(Task::Files(files)) => self.visit(files),
            None => return false,
        };
        if !done {
            self.pool.stop();
            return false;
        }
        self.finish();
        self.share();
        true
    }
}

pub fn crawl<S: Sink>(
    query: &Query,
    pace: Pace,
    threads: &dyn Fn() -> usize,
    make: &(dyn Fn() -> S + Sync),
) -> bool {
    let real = |root: &Path| fs::symlink_metadata(root).is_ok_and(|found| found.is_dir());
    if query.walk.follow || !query.roots.iter().all(|root| real(root)) {
        return false;
    }
    let rules = Rules::new(query);
    let bases: Vec<Base> = query
        .roots
        .iter()
        .map(|root| Base::new(root, rules.active()))
        .collect();
    let roots = query.roots.iter().enumerate().map(|(root, path)| {
        Task::Folder(Folder {
            path: path.as_os_str().as_bytes().to_vec(),
            depth: 0,
            root,
            layer: None,
        })
    });
    let pool = Pool {
        rules,
        bases,
        depth: query.walk.depth,
        batch: pace.batch,
        queue: Mutex::new(roots.rev().collect()),
        wake: Condvar::new(),
        open: AtomicUsize::new(query.roots.len()),
        idle: AtomicUsize::new(0),
        quit: AtomicBool::new(false),
    };
    let started = Instant::now();
    std::thread::scope(|scope| {
        let mut lead = Crew::new(&pool, make());
        let mut alone = true;
        while lead.step() {
            if alone && (lead.seen > pace.few || started.elapsed() > PATIENCE) {
                alone = false;
                for _ in 1..threads() {
                    scope.spawn(|| {
                        let mut crew = Crew::new(&pool, make());
                        while crew.step() {}
                    });
                }
            }
        }
    });
    true
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use ignore::WalkState;

    use super::*;
    use crate::parse::{self, Outcome};
    use crate::walk::walker;

    type Found = (String, String, usize);

    struct Gather<'a> {
        found: &'a Mutex<Vec<Found>>,
    }

    impl Sink for Gather<'_> {
        fn take(&mut self, visit: Visit<'_>) -> bool {
            let found = match visit {
                Visit::Entry(entry) => (
                    entry.path.display().to_string(),
                    format!("{:?}", entry.form),
                    entry.depth,
                ),
                Visit::Failure(text) => (text.to_string(), "failure".to_string(), 0),
                Visit::Passed(_) => return true,
            };
            self.found.lock().expect("a lock").push(found);
            true
        }

        fn heavy(&self, name: &[u8], _: Form) -> bool {
            name.len().is_multiple_of(2)
        }
    }

    struct Dice(u64);

    impl Dice {
        fn roll(&mut self, sides: usize) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as usize % sides
        }

        fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
            choices[self.roll(choices.len())]
        }
    }

    const NAMES: [&str; 14] = [
        "alpha",
        "beta",
        "build",
        "docs",
        "lib",
        "notes.txt",
        "main.rs",
        "data.log",
        ".hidden",
        ".env",
        "node_modules",
        "x.pyc",
        "a b",
        "deep",
    ];
    const RULES: [&str; 12] = [
        "*.log\n",
        "build\n",
        "docs/\n",
        "!notes.txt\n*.txt\n",
        "/lib\n",
        "deep\n",
        "!.hidden\n",
        "alpha/beta\n",
        "**/main.rs\n!/main.rs\n",
        "*\n!*.rs\n!*/\n",
        "[\n*.rs\n",
        "!.env\n!data.log\n",
    ];

    fn grow(folder: &Path, dice: &mut Dice, depth: usize) {
        fs::create_dir_all(folder).expect("create a folder");
        for _ in 0..dice.roll(7) {
            let path = folder.join(dice.pick(&NAMES));
            if path.symlink_metadata().is_ok() {
                continue;
            }
            match dice.roll(10) {
                0..=3 if depth < 4 => grow(&path, dice, depth + 1),
                4 => {
                    let _ = std::os::unix::fs::symlink(dice.pick(&NAMES), &path);
                }
                _ => fs::write(&path, "text\n").expect("write a file"),
            }
        }
        match dice.roll(12) {
            0 | 1 => fs::write(folder.join(".gitignore"), dice.pick(&RULES)).expect("write rules"),
            2 => fs::write(folder.join(".ignore"), dice.pick(&RULES)).expect("write rules"),
            3 if fs::create_dir_all(folder.join(".git/info")).is_ok() => {
                let rules = dice.pick(&RULES);
                fs::write(folder.join(".git/info/exclude"), rules).expect("write rules");
            }
            4 => fs::create_dir_all(folder.join(".jj")).expect("create a folder"),
            5 if !folder.join(".git").exists() => {
                fs::write(folder.join(".git"), "gitdir: /nonexistent\n").expect("write a file");
            }
            _ => {}
        }
    }

    fn folders(root: &Path) -> Vec<PathBuf> {
        let mut found = vec![root.to_path_buf()];
        let mut at = 0;
        while at < found.len() {
            let listing = fs::read_dir(&found[at]).expect("read a folder");
            for entry in listing.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    found.push(entry.path());
                }
            }
            at += 1;
        }
        found
    }

    fn query(flags: &[&str], root: &str) -> Query {
        let mut args: Vec<OsString> = flags.iter().map(OsString::from).collect();
        args.push("-n".into());
        match parse::parse(args) {
            Ok(Outcome::Run(mut query)) => {
                query.roots = vec![PathBuf::from(root)];
                *query
            }
            _ => panic!("expected a search"),
        }
    }

    fn both(flags: &[&str], root: &str) -> Option<(Vec<Found>, Vec<Found>)> {
        let query = query(flags, root);
        let crawled = Mutex::new(Vec::new());
        let pace = Pace {
            few: 8,
            batch: flags.len() % 3,
        };
        if !crawl(&query, pace, &|| 4, &|| Gather { found: &crawled }) {
            return None;
        }
        let walked = Mutex::new(Vec::new());
        walker(&query).build_parallel().run(|| {
            Box::new(|result| {
                let found = match result {
                    Ok(entry) => (
                        entry.path().display().to_string(),
                        format!("{:?}", Form::of(entry.file_type().expect("a kind"))),
                        entry.depth(),
                    ),
                    Err(error) => (error.to_string(), "failure".to_string(), 0),
                };
                walked.lock().expect("a lock").push(found);
                WalkState::Continue
            })
        });
        let sorted = |found: Mutex<Vec<Found>>| {
            let mut found = found.into_inner().expect("a lock");
            found.sort();
            found
        };
        Some((sorted(crawled), sorted(walked)))
    }

    #[test]
    fn the_crawl_visits_what_the_ignore_walker_visits() {
        let base = std::env::temp_dir().join(format!("syk-crawl-{}", std::process::id()));
        let choices: [&[&str]; 9] = [
            &[],
            &["-H"],
            &["-I"],
            &["-a"],
            &["-D", "2"],
            &["-E", "docs"],
            &["-u", "all", "-E", "*.rs"],
            &["-H", "-g", "build", "-D", "3"],
            &["-E", ".hidden", "-E", "alpha"],
        ];
        let mut compared = 0;
        for seed in 0..40 {
            let mut dice = Dice(seed);
            let tree = base.join(format!("tree{seed}"));
            for _ in 0..3 {
                grow(&tree, &mut dice, 0);
            }
            let inside = folders(&tree);
            let locked = (seed % 5 == 0).then(|| inside[dice.roll(inside.len())].clone());
            if let Some(locked) = locked.as_ref().filter(|locked| **locked != tree) {
                fs::set_permissions(locked, fs::Permissions::from_mode(0o000)).expect("lock");
            }
            let roots = [
                format!("{}/", tree.display()),
                inside[dice.roll(inside.len())].display().to_string(),
                inside[dice.roll(inside.len())].display().to_string(),
            ];
            for root in &roots {
                for flags in choices {
                    if let Some((crawled, walked)) = both(flags, root) {
                        assert_eq!(crawled, walked, "seed {seed}, {flags:?} in {root}");
                        compared += crawled.len();
                    }
                }
            }
            if let Some(locked) = locked {
                fs::set_permissions(locked, fs::Permissions::from_mode(0o755)).expect("unlock");
            }
        }
        assert!(compared > 10_000, "only {compared} entries were compared");
        fs::remove_dir_all(base).expect("remove the trees");
    }

    #[test]
    fn a_root_that_is_no_plain_folder_is_left_to_the_other_walker() {
        let base = std::env::temp_dir().join(format!("syk-crawl-root-{}", std::process::id()));
        fs::create_dir_all(base.join("real")).expect("create a folder");
        fs::write(base.join("file"), "text\n").expect("write a file");
        std::os::unix::fs::symlink("real", base.join("link")).expect("link a folder");
        let found = Mutex::new(Vec::new());
        let tried = |flags: &[&str], name: &str| {
            let root = base.join(name).display().to_string();
            let pace = Pace { few: 8, batch: 2 };
            crawl(&query(flags, &root), pace, &|| 2, &|| Gather {
                found: &found,
            })
        };
        assert!(!tried(&[], "file") && !tried(&[], "link") && !tried(&[], "missing"));
        assert!(!tried(&["-L"], "real") && found.lock().expect("a lock").is_empty());
        assert!(tried(&[], "real") && tried(&[], "link/"));
        assert_eq!(found.lock().expect("a lock").len(), 2);
        fs::remove_dir_all(base).expect("remove the folder");
    }
}
