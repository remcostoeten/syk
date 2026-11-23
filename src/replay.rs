use std::collections::HashSet;
use std::fs;
use std::io::{self, IsTerminal};
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use ignore::WalkState;

use crate::build;
use crate::check::{Fresh, Opened};
use crate::group::{self, Skip};
use crate::index::{Index, home, place};
use crate::need::Need;
use crate::query::Query;
use crate::report::counted;
use crate::scan::{self, Scan};
use crate::walk::{Bars, excludes, threads, walker};
use crate::worker::{AHEAD, Shared, Worker};

const PIECE: Range<u32> = 64..4096;
const BIG: u64 = 20_000;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Advice {
    Leave,
    Large,
    Build,
}

pub struct Stored {
    index: Index,
    pub dirty: Vec<u32>,
    fresh: Option<Fresh>,
}

fn print(query: &Query) -> String {
    let view = view(query);
    let mut print = format!("1{}", view.replace(['-', ' '], "").to_uppercase());
    let whole = query
        .filters
        .groups
        .iter()
        .filter(|group| group.skip == Skip::Whole && !view.contains(['I', 'a']));
    for pattern in whole.flat_map(|group| &group.patterns) {
        print.push('\n');
        print.push_str(pattern);
    }
    print
}

fn view(query: &Query) -> &'static str {
    let walk = &query.walk;
    match (walk.all, walk.hidden, walk.no_ignore) {
        (true, _, _) => "-a",
        (false, true, true) => "-H -I",
        (false, true, false) => "-H",
        (false, false, true) => "-I",
        (false, false, false) => "",
    }
}

fn indexable(query: &Query) -> bool {
    !(query.walk.no_index || query.walk.follow) && query.roots.len() == 1
}

fn unasked(query: &Query) -> bool {
    !query.walk.no_ignore && !query.walk.all
}

pub fn stored(query: &Query, sorted: bool) -> (Option<Stored>, Advice) {
    if !indexable(query) {
        return (None, Advice::Leave);
    }
    let root = &query.roots[0];
    let index = match Index::open(root, &print(query)) {
        Opened::Ready(index) => *index,
        Opened::Stale => return (None, Advice::Build),
        Opened::Missing if unasked(query) => return (None, Advice::Large),
        Opened::Missing => return (None, Advice::Leave),
    };
    let bytes = root.as_os_str().as_bytes();
    let dirty = index.stale(bytes, threads(query));
    let limit = (index.within.len() / 16).max(64);
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
    let bars = Bars::new(query);
    let mut builder = walker(query);
    builder.filter_entry({
        let (fresh, added) = (fresh.clone(), added.clone());
        move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if bars.bar(entry) {
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

pub fn replay(shared: &Shared, query: &Query, stored: Stored, sorted: bool) {
    let index = &stored.index;
    let mut state = vec![0u8; index.folders() as usize];
    for slot in &stored.dirty {
        state[*slot as usize] |= scan::DIRTY;
    }
    if let Some(fresh) = stored.fresh {
        for slot in refresh(shared, query, fresh) {
            state[slot as usize] |= scan::GONE;
        }
    }
    let excluded = excludes(query).ok().filter(|excluded| !excluded.is_empty());
    let gate = shared
        .gate()
        .map(|pattern| move |names: &[u8], at: usize| pattern.next_name(names, at));
    let need = shared.need().filter(|need| *need != Need::Any);
    let signs = need.as_ref().and_then(|_| index.signs());
    let verdicts = signs.as_ref().zip(need.as_ref());
    let verdicts = verdicts.map(|(signs, need)| signs.judge(need));
    let scan = Scan {
        index,
        root: query.roots[0].as_os_str().as_bytes(),
        depth: query.walk.depth,
        state: &state,
        excluded: excluded.as_ref(),
        gate: gate.as_ref().map(|gate| gate as &scan::Gate<'_>),
        verdicts: verdicts.as_ref(),
    };
    let span = index.span.clone();
    let total = span.end - span.start;
    if sorted {
        let mut worker = Worker::new(shared, 0);
        scan.run(span, &mut |visit| worker.take(visit));
        return;
    }
    let next = AtomicU32::new(span.start);
    let size = (total / (threads(query) as u32 * 8).max(1)).clamp(PIECE.start, PIECE.end);
    let crew = threads(query).clamp(1, total.div_ceil(size) as usize);
    std::thread::scope(|scope| {
        for _ in 0..crew {
            scope.spawn(|| {
                let mut worker = Worker::new(shared, AHEAD);
                let mut going = true;
                while going && !shared.budget.stopped() {
                    let from = next.fetch_add(size, Ordering::Relaxed);
                    if from >= span.end {
                        break;
                    }
                    let piece = from..from.saturating_add(size).min(span.end);
                    going = scan.run(piece, &mut |visit| worker.take(visit));
                }
            });
        }
    });
}

pub fn reindex(mut query: Query, started: Instant) -> ExitCode {
    let query = &mut query;
    let plain = indexable(query) && query.filters.excludes.is_empty() && query.walk.depth.is_none();
    if !plain {
        eprintln!(
            "syk: an index holds everything one search of one folder sees, so -y does not go with -L, -E, -D, -Y or several paths."
        );
        return ExitCode::from(2);
    }
    let shown = query.roots[0].clone();
    if let Ok(real) = fs::canonicalize(&shown) {
        query.roots[0] = real;
    }
    let root = &query.roots[0];
    match build::build(
        root,
        &print(query),
        walker(query),
        Bars::new(query),
        threads(query),
    ) {
        Ok(built) => {
            println!(
                "Indexed {} in {} and the contents of {} below {} in {:.2} s.",
                counted(built.entries as u64, "entry", "entries"),
                counted(built.folders as u64, "folder", "folders"),
                counted(built.signed as u64, "file", "files"),
                shown.display(),
                started.elapsed().as_secs_f64()
            );
            if built.kept > 0 {
                println!(
                    "{} had not changed and {} not read again.",
                    counted(built.kept as u64, "file", "files"),
                    if built.kept == 1 { "was" } else { "were" }
                );
            }
            ExitCode::SUCCESS
        }
        Err(reason) => {
            eprintln!("syk: no index for {}: {reason}.", shown.display());
            ExitCode::from(2)
        }
    }
}

fn homeward(query: &Query, real: &Path, print: &str) -> Option<PathBuf> {
    let home = fs::canonicalize(home()?).ok()?;
    let fits = view(query).is_empty()
        && io::stderr().is_terminal()
        && home.parent().is_some()
        && real.starts_with(&home);
    let absent = place(&home, print, "idx").is_some_and(|index| !index.exists());
    (fits && absent && build::wanted(&home, print) && !build::unplugged()).then_some(home)
}

fn tell(home: &Path) {
    let Ok(told) = build::ready().map(|store| store.join("told")) else {
        return;
    };
    if !told.exists() && fs::write(&told, "").is_ok() {
        eprintln!(
            "syk: building an index of {} in the background, so that later searches are faster. syk -A lists the indexes, and -Y in the config file switches them off.",
            home.display()
        );
    }
}

fn same(query: &Query) -> Vec<String> {
    let mut flags = vec!["-U".to_string(), "-u".to_string(), group::EVERY.to_string()];
    flags.extend(view(query).split(' ').map(str::to_string));
    flags.retain(|flag| !flag.is_empty());
    let whole = query
        .filters
        .groups
        .iter()
        .filter(|group| group.skip == Skip::Whole);
    for group in whole {
        flags.push("-g".to_string());
        flags.push(format!("{}={}", group.name, group.patterns.join(",")));
    }
    flags
}

pub fn background(query: &Query, advice: Advice, worn: bool, entries: u64) {
    let renew = advice == Advice::Build || worn;
    if !indexable(query) || !(renew || advice == Advice::Large) {
        return;
    }
    let print = print(query);
    let Ok(real) = fs::canonicalize(&query.roots[0]) else {
        return;
    };
    let home = match (renew, advice) {
        (false, Advice::Large) => homeward(query, &real, &print),
        _ => None,
    };
    let root = match (renew, advice, &home) {
        (true, _, _) => Index::cover(&real, &print).unwrap_or(real),
        (false, Advice::Large, Some(home)) => home.clone(),
        (false, Advice::Large, None) if entries >= BIG => real,
        _ => return,
    };
    if !build::wanted(&root, &print) || build::busy() || build::cramped() {
        return;
    }
    let mut folder = root.into_os_string();
    folder.push("/");
    if build::spawn(Path::new(&folder), &same(query)).is_ok()
        && let Some(home) = home
    {
        tell(&home);
    }
}
