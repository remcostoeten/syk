use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};

use grep_matcher::Matcher;
use grep_searcher::{Searcher, Sink, SinkContext, SinkMatch};

use crate::stop;

const RESET: &[u8] = b"\x1b[0m";
const DIM: &[u8] = b"\x1b[2m";
const BOLD: &[u8] = b"\x1b[1m";
const HIT: &[u8] = b"\x1b[1;31m";
const FOLDER: &[u8] = b"\x1b[1;34m";
const NUMBER: &[u8] = b"\x1b[32m";
const LINK: &[u8] = b"\x1b]8;;";
const LINK_END: &[u8] = b"\x1b\\";
const CUT: &[u8] = "…".as_bytes();
const LONG_LINE: usize = 320;
const LEAD: usize = 80;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    List,
    Count,
    Flat,
    Grouped,
}

#[derive(Clone, Copy)]
pub struct Style<'a> {
    pub color: bool,
    pub json: bool,
    pub end: u8,
    pub base: Option<&'a [u8]>,
}

pub struct Budget {
    limited: bool,
    left: AtomicI64,
    stopped: AtomicBool,
}

impl Budget {
    pub fn new(max: Option<usize>) -> Budget {
        Budget {
            limited: max.is_some(),
            left: AtomicI64::new(max.unwrap_or(0) as i64),
            stopped: AtomicBool::new(max == Some(0)),
        }
    }

    pub fn take(&self) -> bool {
        if self.stopped() {
            return false;
        }
        if !self.limited {
            return true;
        }
        let before = self.left.fetch_sub(1, Ordering::Relaxed);
        if before <= 1 {
            self.stop();
        }
        before >= 1
    }

    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Relaxed);
    }

    pub fn stopped(&self) -> bool {
        self.stopped.load(Ordering::Relaxed) || stop::asked()
    }
}

pub struct Entry<'a> {
    pub path: &'a [u8],
    pub name_at: usize,
    pub hit: Option<(usize, usize)>,
    pub folder: bool,
}

fn link(out: &mut Vec<u8>, base: &[u8], path: &[u8]) {
    out.extend_from_slice(LINK);
    out.extend_from_slice(b"file://");
    let joined = [base, b"/", path];
    let parts: &[&[u8]] = if path.starts_with(b"/") {
        &joined[2..]
    } else {
        &joined
    };
    for byte in parts.iter().flat_map(|part| part.iter()) {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.' | b'~') {
            out.push(*byte);
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out.extend_from_slice(LINK_END);
}

fn label(out: &mut Vec<u8>, entry: &Entry<'_>, style: Style<'_>) {
    if !style.color {
        out.extend_from_slice(entry.path);
    } else {
        if let Some(base) = style.base {
            link(out, base, entry.path);
        }
        let (parent, name) = entry.path.split_at(entry.name_at.min(entry.path.len()));
        let tone = if entry.folder { FOLDER } else { BOLD };
        if !parent.is_empty() {
            paint(out, DIM, parent);
        }
        match entry.hit {
            Some((start, end)) if end <= name.len() => {
                paint(out, tone, &name[..start]);
                paint(out, HIT, &name[start..end]);
                paint(out, tone, &name[end..]);
            }
            _ => paint(out, tone, name),
        }
        if style.base.is_some() {
            out.extend_from_slice(LINK);
            out.extend_from_slice(LINK_END);
        }
    }
    if entry.folder {
        out.push(b'/');
    }
}

pub fn write_entry(out: &mut Vec<u8>, entry: &Entry<'_>, style: Style<'_>) {
    if style.json {
        out.extend_from_slice(b"{\"type\":\"path\",");
        located(out, entry.path);
        let kind = if entry.folder { "dir" } else { "file" };
        let _ = writeln!(out, ",\"kind\":\"{kind}\"}}");
        return;
    }
    label(out, entry, style);
    out.push(style.end);
}

pub fn write_count(out: &mut Vec<u8>, entry: &Entry<'_>, count: u64, style: Style<'_>) {
    if style.json {
        out.extend_from_slice(b"{\"type\":\"count\",");
        located(out, entry.path);
        let _ = writeln!(out, ",\"count\":{count}}}");
        return;
    }
    label(out, entry, style);
    out.push(b':');
    if style.color {
        paint(out, NUMBER, count.to_string().as_bytes());
    } else {
        let _ = write!(out, "{count}");
    }
    out.push(style.end);
}

fn located(out: &mut Vec<u8>, path: &[u8]) {
    out.extend_from_slice(b"\"path\":");
    quote(out, path);
    if std::str::from_utf8(path).is_err() {
        out.extend_from_slice(b",\"path_hex\":\"");
        for byte in path {
            let _ = write!(out, "{byte:02x}");
        }
        out.push(b'"');
    }
}

fn quote(out: &mut Vec<u8>, bytes: &[u8]) {
    out.push(b'"');
    for character in String::from_utf8_lossy(bytes).chars() {
        match character {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            control if (control as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", control as u32);
            }
            other => {
                let mut buffer = [0u8; 4];
                out.extend_from_slice(other.encode_utf8(&mut buffer).as_bytes());
            }
        }
    }
    out.push(b'"');
}

fn paint(out: &mut Vec<u8>, style: &[u8], text: &[u8]) {
    if text.is_empty() {
        return;
    }
    out.extend_from_slice(style);
    out.extend_from_slice(text);
    out.extend_from_slice(RESET);
}

pub struct Lines<'a, M> {
    pub out: &'a mut Vec<u8>,
    pub entry: &'a Entry<'a>,
    pub matcher: &'a M,
    pub layout: Layout,
    pub style: Style<'a>,
    pub budget: &'a Budget,
    pub headed: bool,
    pub hits: u64,
    pub first: Option<u64>,
    pub binary: bool,
}

impl<M: Matcher> Lines<'_, M> {
    fn record(&mut self, number: Option<u64>, bytes: &[u8], matched: bool) {
        let line = trim(bytes);
        let kind = if matched { "match" } else { "context" };
        let _ = write!(self.out, "{{\"type\":\"{kind}\",");
        located(self.out, self.entry.path);
        let _ = write!(self.out, ",\"line\":{},\"text\":", number.unwrap_or(0));
        quote(self.out, line);
        if matched {
            self.out.extend_from_slice(b",\"spans\":[");
            let out = &mut *self.out;
            let mut first = true;
            let _ = self.matcher.find_iter(line, |found| {
                if !found.is_empty() {
                    let comma = if first { "" } else { "," };
                    let _ = write!(out, "{comma}[{},{}]", found.start(), found.end());
                    first = false;
                }
                true
            });
            self.out.push(b']');
        }
        self.out.extend_from_slice(b"}\n");
    }

    fn line(&mut self, number: Option<u64>, bytes: &[u8], separator: u8, matched: bool) {
        if self.style.json {
            return self.record(number, bytes, matched);
        }
        match self.layout {
            Layout::List | Layout::Count => return,
            Layout::Flat => {
                self.out.extend_from_slice(self.entry.path);
                self.out.push(separator);
            }
            Layout::Grouped if !self.headed => {
                write_entry(self.out, self.entry, self.style);
                self.headed = true;
            }
            Layout::Grouped => {}
        }
        let number = number.unwrap_or(0);
        if self.style.color {
            self.out
                .extend_from_slice(if matched { NUMBER } else { DIM });
            let _ = write!(self.out, "{number}");
            self.out.extend_from_slice(RESET);
        } else {
            let _ = write!(self.out, "{number}");
        }
        self.out.push(separator);
        self.text(bytes, matched);
        self.out.push(b'\n');
    }

    fn text(&mut self, bytes: &[u8], matched: bool) {
        let line = trim(bytes);
        let clip = self.layout == Layout::Grouped && line.len() > LONG_LINE;
        if !clip && !(self.style.color && matched) {
            self.out.extend_from_slice(line);
            return;
        }
        let (from, to) = if clip {
            let anchor = match self.matcher.find(line) {
                Ok(Some(found)) if matched => found.start(),
                _ => 0,
            };
            let from = floor(line, anchor.saturating_sub(LEAD));
            (from, ceil(line, (from + LONG_LINE).min(line.len())))
        } else {
            (0, line.len())
        };
        if from > 0 {
            self.out.extend_from_slice(CUT);
        }
        let mut at = from;
        if self.style.color && matched {
            let out = &mut *self.out;
            let _ = self.matcher.find_iter(line, |found| {
                if found.start() >= to {
                    return false;
                }
                if found.is_empty() || found.end() <= at {
                    return true;
                }
                let start = found.start().max(at);
                let end = found.end().min(to);
                out.extend_from_slice(&line[at..start]);
                paint(out, HIT, &line[start..end]);
                at = end;
                true
            });
        }
        self.out.extend_from_slice(&line[at..to]);
        if to < line.len() {
            self.out.extend_from_slice(CUT);
        }
    }
}

impl<M: Matcher> Sink for Lines<'_, M> {
    type Error = io::Error;

    fn matched(&mut self, _: &Searcher, found: &SinkMatch<'_>) -> Result<bool, io::Error> {
        if self.first.is_none() {
            self.first = found.line_number();
        }
        match self.layout {
            Layout::List => {
                self.hits = 1;
                return Ok(false);
            }
            Layout::Count => {
                self.hits += 1;
                return Ok(true);
            }
            Layout::Flat | Layout::Grouped => {}
        }
        if !self.budget.take() {
            return Ok(false);
        }
        self.hits += 1;
        self.line(found.line_number(), found.bytes(), b':', true);
        Ok(!self.budget.stopped())
    }

    fn context(&mut self, _: &Searcher, context: &SinkContext<'_>) -> Result<bool, io::Error> {
        self.line(context.line_number(), context.bytes(), b'-', false);
        Ok(true)
    }

    fn binary_data(&mut self, _: &Searcher, _: u64) -> Result<bool, io::Error> {
        self.binary = true;
        Ok(false)
    }

    fn context_break(&mut self, _: &Searcher) -> Result<bool, io::Error> {
        let lines = matches!(self.layout, Layout::Flat | Layout::Grouped);
        if lines && !self.style.json {
            self.out.extend_from_slice(b"--\n");
        }
        Ok(true)
    }
}

fn trim(mut line: &[u8]) -> &[u8] {
    while let [rest @ .., b'\n' | b'\r'] = line {
        line = rest;
    }
    line
}

fn continues(byte: u8) -> bool {
    byte & 0b1100_0000 == 0b1000_0000
}

fn floor(line: &[u8], mut at: usize) -> usize {
    while at > 0 && continues(line[at]) {
        at -= 1;
    }
    at
}

fn ceil(line: &[u8], mut at: usize) -> usize {
    while at < line.len() && continues(line[at]) {
        at += 1;
    }
    at
}
