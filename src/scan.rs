use std::ffi::OsStr;
use std::ops::Range;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use ignore::overrides::Override;

use crate::index::{Index, Record, STRIDE, Trail};
use crate::item::{Form, Item};
use crate::sign::{Ruled, Verdicts};

const DENSE: u32 = 8;

pub const DIRTY: u8 = 1;
pub const GONE: u8 = 2;

pub enum Visit<'a> {
    Entry(Item<'a>),
    Failure(&'a str),
    Passed(u64),
}

pub type Gate<'a> = dyn Fn(&[u8], usize) -> Option<(usize, usize)> + Sync + 'a;

pub struct Scan<'a> {
    pub index: &'a Index,
    pub root: &'a [u8],
    pub depth: Option<usize>,
    pub state: &'a [u8],
    pub excluded: Option<&'a Override>,
    pub gate: Option<&'a Gate<'a>>,
    pub verdicts: Option<&'a Verdicts<'a>>,
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

    fn ruling(&self, at: u32, record: &Record, hint: &mut usize) -> Option<Ruled> {
        match (self.verdicts, record.form()) {
            (Some(verdicts), Form::File) => verdicts.ruling(at, hint),
            _ => None,
        }
    }

    fn entry<'a>(&'a self, record: &Record, trail: &'a Trail, ruled: Option<Ruled>) -> Item<'a> {
        Item {
            path: trail.as_path(),
            name: match record.depth {
                0 => trail
                    .as_path()
                    .file_name()
                    .unwrap_or(trail.as_path().as_os_str()),
                _ => OsStr::from_bytes(self.index.name(record)),
            },
            depth: record.depth as usize,
            form: record.form(),
            follow: false,
            ruled,
        }
    }

    fn failed(&self, at: u32, record: &Record, failure: &mut usize) -> Vec<String> {
        let index = self.index;
        let state = self.state.get(record.slot() as usize).copied().unwrap_or(0);
        let read = self
            .depth
            .is_none_or(|deepest| (record.depth as usize) < deepest);
        let mut messages = Vec::new();
        while let Some((entry, text)) = index.failures.get(*failure)
            && *entry <= at
        {
            if *entry == at && state & DIRTY == 0 && read {
                messages.push(index.failure(text.clone(), self.root));
            }
            *failure += 1;
        }
        messages
    }

    fn reach(&self, at: u32, trail: &mut Trail) -> Option<Record> {
        let index = self.index;
        for above in index.chain(at).into_iter().chain([at]) {
            let record = index.record(above);
            trail.step(self.root, record.depth as usize, index.name(&record));
            if self.skipped(&record, trail.as_path()) {
                return None;
            }
        }
        Some(index.record(at))
    }

    fn locate(&self, entries: &Range<u32>, offset: usize) -> u32 {
        let (mut low, mut high) = (entries.start, entries.end);
        while high - low > 1 {
            let middle = low + (high - low) / 2;
            match self.index.record(middle).name as usize <= offset {
                true => low = middle,
                false => high = middle,
            }
        }
        low
    }

    fn sift(
        &self,
        gate: &Gate<'_>,
        entries: Range<u32>,
        visit: &mut dyn FnMut(Visit<'_>) -> bool,
    ) -> bool {
        let index = self.index;
        let names = &index.map[index.names.clone()];
        let start = index.record(entries.start).name as usize;
        let stop = match entries.end < index.entries {
            true => index.record(entries.end).name as usize,
            false => names.len(),
        };
        let Some(blob) = names.get(start..stop) else {
            return true;
        };
        let mut trail = Trail::new();
        let (mut at, mut hits) = (0, 0);
        let mut hint = 0;
        let mut done = entries.start;
        while let Some((line, _)) = gate(blob, at) {
            let entry = self.locate(&entries, start + line);
            let record = index.record(entry);
            let after = record.name as usize + record.len as usize + 1;
            at = after.saturating_sub(start).max(at + 1);
            hits += 1;
            if let Some(record) = self.reach(entry, &mut trail) {
                let ruled = self.ruling(entry, &record, &mut hint);
                if !visit(Visit::Entry(self.entry(&record, &trail, ruled))) {
                    return false;
                }
            }
            done = entry + 1;
            if hits >= STRIDE && hits * DENSE > done - entries.start {
                break;
            }
        }
        let sifted = match hits >= STRIDE && hits * DENSE > done - entries.start {
            true => entries.start..done,
            false => entries.clone(),
        };
        let mut failure = index.failures(sifted.start);
        while let Some((entry, _)) = index.failures.get(failure).cloned()
            && entry < sifted.end
        {
            let messages = match self.reach(entry, &mut trail) {
                Some(record) => self.failed(entry, &record, &mut failure),
                None => Vec::new(),
            };
            failure = failure.max(index.failures(entry + 1));
            if !messages
                .iter()
                .all(|message| visit(Visit::Failure(message)))
            {
                return false;
            }
        }
        let passed = u64::from(sifted.end - sifted.start).saturating_sub(u64::from(hits));
        visit(Visit::Passed(passed))
            && (sifted.end == entries.end || self.plod(sifted.end..entries.end, visit))
    }

    fn plod(&self, entries: Range<u32>, visit: &mut dyn FnMut(Visit<'_>) -> bool) -> bool {
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
        let mut hint = 0;
        while at < entries.end {
            let record = index.record(at);
            trail.step(self.root, record.depth as usize, index.name(&record));
            if self.skipped(&record, trail.as_path()) {
                at = match record.form() {
                    Form::Dir => record.end.max(at + 1),
                    _ => at + 1,
                };
                continue;
            }
            let ruled = self.ruling(at, &record, &mut hint);
            if !visit(Visit::Entry(self.entry(&record, &trail, ruled))) {
                return false;
            }
            let messages = self.failed(at, &record, &mut failure);
            if !messages
                .iter()
                .all(|message| visit(Visit::Failure(message)))
            {
                return false;
            }
            at += 1;
        }
        true
    }

    pub fn run(&self, entries: Range<u32>, visit: &mut dyn FnMut(Visit<'_>) -> bool) -> bool {
        let entries = entries.start..entries.end.min(self.index.entries);
        if entries.is_empty() {
            return true;
        }
        match self.gate {
            Some(gate) => self.sift(gate, entries, visit),
            None => self.plod(entries, visit),
        }
    }
}
