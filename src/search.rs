use std::io::{self, IsTerminal};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use ignore::{WalkBuilder, WalkState};

use crate::group::Skip;
use crate::output::{Budget, Layout};
use crate::plan::Compiled;
use crate::query::{Format, Kind, Query};
use crate::rank::{self, Block};
use crate::replay::{Advice, replay, stored};
use crate::spell::Seen;
use crate::walk::{skips, threads, walker};
use crate::worker::{AHEAD, Found, Shared, Tally, Worker};

const FEW: usize = 128;
const PATIENCE: Duration = Duration::from_millis(2);

#[derive(Clone, Copy)]
pub struct View {
    pub layout: Layout,
    pub color: bool,
    pub live: bool,
    pub json: bool,
    pub end: u8,
    pub links: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pass {
    First,
    Gather { fold: bool },
    Again,
}

pub struct Outcome {
    pub found: bool,
    pub failed: bool,
    pub seen: Seen,
    pub blocks: Vec<Block>,
    pub tally: Tally,
    pub advice: Advice,
    pub reread: Option<usize>,
}

pub fn view(query: &Query) -> View {
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

#[cfg(target_os = "linux")]
fn crawled(shared: &Shared, query: &Query) -> bool {
    let (few, batch) = match (shared.plan.content().is_some(), shared.stat) {
        (true, _) => (FEW, 1),
        (false, true) => (FEW, 8),
        (false, false) => (FEW * 8, 0),
    };
    let pace = crate::crawl::Pace {
        few: few as u64,
        batch,
    };
    crate::crawl::crawl(query, pace, &|| threads(query), &|| {
        Worker::new(shared, AHEAD)
    })
}

#[cfg(not(target_os = "linux"))]
fn crawled(_: &Shared, _: &Query) -> bool {
    false
}

fn shared(query: &Query, plan: Compiled, view: View, pass: Pass, keep: bool) -> Shared {
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
    Shared {
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
    }
}

fn walk(shared: &Shared, query: &Query, sorted: bool) {
    if sorted {
        let mut builder = walker(query);
        builder.sort_by_file_name(|left, right| left.cmp(right));
        let mut worker = Worker::new(shared, 0);
        for result in builder.build() {
            if matches!(worker.visit(result), WalkState::Quit) {
                break;
            }
        }
    } else if !crawled(shared, query) {
        let builder = walker(query);
        match few(&builder) {
            Some(found) => handful(shared, found, threads(query)),
            None => builder.build_parallel().run(|| {
                let mut worker = Worker::new(shared, AHEAD);
                Box::new(move |result| worker.visit(result))
            }),
        }
    }
}

pub fn search(query: &Query, plan: Compiled, view: View, pass: Pass, keep: bool) -> Outcome {
    let shared = shared(query, plan, view, pass, keep);
    let sorted = query.walk.sort && !shared.gather;
    let (stored, advice) = stored(query, sorted);
    let reread = stored.as_ref().map(|stored| stored.dirty.len());
    match stored {
        Some(stored) => replay(&shared, query, stored, sorted),
        None => walk(&shared, query, sorted),
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
