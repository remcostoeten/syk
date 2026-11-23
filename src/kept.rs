use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::{self, Metadata};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use ignore::overrides::Override;

use crate::group::Skip;
use crate::output::{Entry, Style, write_entry};
use crate::pattern::Pattern;
use crate::query::{Kind, Plan, Query, Targets};
use crate::rank::{self, Block, Habits, Score};
use crate::search::View;
use crate::walk::{excludes, skips};
use crate::worker::CHUNK;

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

fn below<'a>(roots: &[Vec<u8>], path: &'a [u8]) -> Option<&'a [u8]> {
    roots.iter().find_map(|root| {
        let rest = path.strip_prefix(root.as_slice())?;
        let rest = if root.len() > 1 {
            rest.strip_prefix(b"/")?
        } else {
            rest
        };
        (!rest.is_empty()).then_some(rest)
    })
}

fn asked(query: &Query, targets: &Targets, metadata: &Metadata) -> bool {
    let folder = metadata.is_dir();
    let wanted = match query.filters.kind {
        None => folder || metadata.is_file(),
        Some(Kind::Dir) => folder,
        Some(Kind::File) => metadata.is_file(),
        Some(Kind::Link) => false,
    };
    let target = if folder { targets.dirs } else { targets.files };
    wanted && target
}

fn left_out(excluded: &Override, shown: &[u8], folder: bool) -> bool {
    Path::new(OsStr::from_bytes(shown))
        .ancestors()
        .filter(|part| !part.as_os_str().is_empty())
        .enumerate()
        .any(|(above, part)| excluded.matched(part, above > 0 || folder).is_ignore())
}

fn block(
    shown: &[u8],
    name: &[u8],
    hit: (usize, usize),
    depth: usize,
    metadata: &Metadata,
    style: Style<'_>,
) -> Block {
    let mut text = Vec::new();
    let entry = Entry {
        path: shown,
        name_at: shown.len().saturating_sub(name.len()),
        hit: Some(hit),
        folder: metadata.is_dir(),
    };
    write_entry(&mut text, &entry, style);
    Block {
        path: shown.to_vec(),
        quality: rank::quality(name, Some(hit)),
        depth,
        modified: metadata.modified().ok(),
        line: None,
        text,
    }
}

pub fn remembered(query: &Query, habits: &Habits, found: &[Block], view: View) -> Vec<Block> {
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
        let Some(below) = below(&roots, path) else {
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
        if !asked(query, targets, &metadata) {
            continue;
        }
        let folder = metadata.is_dir();
        let shown = habits.relative(path);
        if let Some(excluded) = &excluded
            && left_out(excluded, shown, folder)
        {
            continue;
        }
        let block = block(shown, name, hit, depth, &metadata, style);
        have.insert(path.to_vec());
        extra.push(block);
    }
    extra
}

pub fn arrange(blocks: Vec<Block>, habits: Option<&Habits>, query: &Query) -> Vec<(Score, Block)> {
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

pub fn show(mut blocks: Vec<(Score, Block)>, query: &Query, view: View) {
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
