use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem::MaybeUninit;
use std::ops::Range;
use std::os::fd::{AsFd, AsRawFd};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicPtr, Ordering};

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const DIM: &str = "\x1b[2m";
pub const PLACE: &str = "\x1b[1;36m";

pub const ENTER: &str = "\x1b[?1049h\x1b[?25l\x1b[?7l";
pub const LEAVE: &str = "\x1b[?7h\x1b[?25h\x1b[?1049l";
const HOME: &str = "\x1b[H";
const CLEAR_LINE: &str = "\x1b[K";
pub const HIT: &str = "\x1b[7m";
pub const HIT_END: &str = "\x1b[27m";
const CHOSEN: &str = "\x1b[7;4m";
const CHOSEN_END: &str = "\x1b[27;24m";

pub const MARGIN: &str = "  ";
const INDENT: &str = "    ";
const SPACE: &str = "   ";
const WIDTH: usize = 80;
const ESCAPE_WAIT: i32 = 50;
const RESIZE_WAIT: i32 = 200;
const CONTEXT: usize = 2;
const LETTERS: &str = "abcdefghijklmnopqrstuvwxyz";
pub const FALLBACK: Size = Size {
    rows: 24,
    columns: WIDTH,
};

const KEYS: [(&str, &str); 9] = [
    (
        "n  Space",
        "the next screen, or the next match while searching",
    ),
    ("b", "one screen back, or the previous match"),
    ("j  k  arrows", "one line down or up"),
    ("g  G", "the first and the last screen"),
    ("/", "search this page"),
    ("s", "jump to a section"),
    ("-", "open the page of a flag, by its letter"),
    ("Esc", "clear the search, close a flag page, or quit"),
    ("q", "quit and leave the screen in view"),
];

static OFF: AtomicBool = AtomicBool::new(false);
static SAVED: AtomicPtr<libc::termios> = AtomicPtr::new(std::ptr::null_mut());
static TTY: AtomicI32 = AtomicI32::new(-1);
const FATAL: [libc::c_int; 4] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT];

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Size {
    pub rows: usize,
    pub columns: usize,
}

pub struct Mark {
    pub line: usize,
    pub title: String,
}

pub struct Sheet {
    pub text: String,
    pub marks: Vec<Mark>,
}

pub struct Setup<'a> {
    pub styled: bool,
    pub open: Option<&'a dyn Fn(char) -> Option<Sheet>>,
}

#[derive(Clone, Copy)]
struct Bar {
    columns: usize,
    styled: bool,
    nested: bool,
    flags: bool,
}

pub fn size(stream: &impl AsFd) -> Option<Size> {
    let mut window = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let status = unsafe { libc::ioctl(stream.as_fd().as_raw_fd(), libc::TIOCGWINSZ, &mut window) };
    (status == 0 && window.ws_row > 0 && window.ws_col > 0).then(|| Size {
        rows: usize::from(window.ws_row),
        columns: usize::from(window.ws_col),
    })
}

pub fn turn_off() {
    OFF.store(true, Ordering::Relaxed);
}

fn disabled() -> bool {
    let off = std::env::var_os("SYK_PAGER").is_some_and(|value| value == "off" || value == "0");
    off || OFF.load(Ordering::Relaxed)
        || std::env::var_os("TERM").is_some_and(|term| term == "dumb")
}

pub fn needed(text: &str) -> bool {
    if disabled() {
        return false;
    }
    size(&io::stdout()).is_some_and(|size| {
        text.lines().count() >= size.rows
            && text
                .lines()
                .all(|line| bare(line).chars().count() <= size.columns)
    })
}

pub fn bare(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut styling = false;
    for letter in line.chars() {
        if styling {
            styling = letter != 'm';
        } else if letter == '\x1b' {
            styling = true;
        } else {
            plain.push(letter);
        }
    }
    plain
}

extern "C" fn restore(signal: libc::c_int) {
    let saved = SAVED.load(Ordering::Relaxed);
    let tty = TTY.load(Ordering::Relaxed);
    unsafe {
        if !saved.is_null() {
            libc::tcsetattr(tty, libc::TCSANOW, saved);
            libc::write(tty, LEAVE.as_ptr().cast(), LEAVE.len());
        }
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

pub struct Raw {
    pub tty: File,
    saved: libc::termios,
}

impl Raw {
    pub fn enter() -> io::Result<Raw> {
        let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
        let mut saved = MaybeUninit::<libc::termios>::uninit();
        if unsafe { libc::tcgetattr(tty.as_raw_fd(), saved.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let saved = unsafe { saved.assume_init() };
        let mut raw = saved;
        raw.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ISIG);
        raw.c_cc[libc::VMIN] = 1;
        raw.c_cc[libc::VTIME] = 0;
        if unsafe { libc::tcsetattr(tty.as_raw_fd(), libc::TCSADRAIN, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        TTY.store(tty.as_raw_fd(), Ordering::Relaxed);
        SAVED.store(Box::into_raw(Box::new(saved)), Ordering::Relaxed);
        let handler = restore as extern "C" fn(libc::c_int);
        for signal in FATAL {
            unsafe { libc::signal(signal, handler as libc::sighandler_t) };
        }
        Ok(Raw { tty, saved })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        for signal in FATAL {
            unsafe { libc::signal(signal, libc::SIG_DFL) };
        }
        SAVED.store(std::ptr::null_mut(), Ordering::Relaxed);
        unsafe { libc::tcsetattr(self.tty.as_raw_fd(), libc::TCSADRAIN, &self.saved) };
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Next,
    Back,
    PageDown,
    PageUp,
    Down,
    Up,
    First,
    Last,
    Search,
    Sections,
    Keys,
    Flag,
    Enter,
    Escape,
    Quit,
    Other,
}

pub enum Typed {
    Byte(u8),
    Erase,
    Wipe,
    Submit,
    Cancel,
    Other,
}

fn waiting(tty: &File, timeout: i32) -> bool {
    let descriptor = tty.as_raw_fd();
    let mut wait = libc::timeval {
        tv_sec: (timeout / 1000) as _,
        tv_usec: ((timeout % 1000) * 1000) as _,
    };
    // macOS cannot poll a terminal device, select works on both systems
    unsafe {
        let mut watched: libc::fd_set = std::mem::zeroed();
        libc::FD_ZERO(&mut watched);
        libc::FD_SET(descriptor, &mut watched);
        let none = std::ptr::null_mut();
        libc::select(descriptor + 1, &mut watched, none, none, &mut wait) > 0
    }
}

fn byte(tty: &mut File) -> io::Result<u8> {
    let mut one = [0u8; 1];
    tty.read_exact(&mut one)?;
    Ok(one[0])
}

fn named(sequence: &[u8]) -> Key {
    match sequence {
        b"[A" | b"OA" => Key::Up,
        b"[B" | b"OB" => Key::Down,
        b"[5~" => Key::PageUp,
        b"[6~" => Key::PageDown,
        b"[H" | b"OH" | b"[1~" => Key::First,
        b"[F" | b"OF" | b"[4~" => Key::Last,
        _ => Key::Other,
    }
}

fn escape(tty: &mut File) -> io::Result<Key> {
    if !waiting(tty, ESCAPE_WAIT) {
        return Ok(Key::Escape);
    }
    let mut sequence = Vec::with_capacity(4);
    while waiting(tty, ESCAPE_WAIT) {
        let next = byte(tty)?;
        sequence.push(next);
        if sequence.len() > 1 && (next.is_ascii_alphabetic() || next == b'~') {
            break;
        }
    }
    Ok(named(&sequence))
}

pub fn key(tty: &mut File) -> io::Result<Key> {
    Ok(match byte(tty)? {
        b'n' => Key::Next,
        b'b' | b'N' => Key::Back,
        b' ' | b'f' => Key::PageDown,
        b'p' => Key::PageUp,
        b'j' => Key::Down,
        b'k' => Key::Up,
        b'g' => Key::First,
        b'G' => Key::Last,
        b'/' => Key::Search,
        b's' => Key::Sections,
        b'?' => Key::Keys,
        b'-' => Key::Flag,
        b'\n' | b'\r' => Key::Enter,
        b'q' | 3 | 4 => Key::Quit,
        0x1b => escape(tty)?,
        _ => Key::Other,
    })
}

pub fn typed(tty: &mut File) -> io::Result<Typed> {
    Ok(match byte(tty)? {
        b'\n' | b'\r' => Typed::Submit,
        0x7f | 0x08 => Typed::Erase,
        0x15 => Typed::Wipe,
        3 => Typed::Cancel,
        0x1b => match escape(tty)? {
            Key::Escape => Typed::Cancel,
            _ => Typed::Other,
        },
        letter if letter >= b' ' => Typed::Byte(letter),
        _ => Typed::Other,
    })
}

fn letter(tty: &mut File) -> io::Result<Option<char>> {
    Ok(match byte(tty)? {
        0x1b => {
            escape(tty)?;
            None
        }
        letter if letter.is_ascii_graphic() => Some(char::from(letter)),
        _ => None,
    })
}

pub fn erase(draft: &mut Vec<u8>) {
    while draft.pop().is_some_and(|last| last & 0xC0 == 0x80) {}
}

pub fn resized(tty: &File, shown: Size) -> bool {
    while !waiting(tty, RESIZE_WAIT) {
        if size(tty).unwrap_or(FALLBACK) != shown {
            return true;
        }
    }
    false
}

struct Search {
    query: String,
    lines: Vec<usize>,
    at: usize,
}

impl Search {
    fn line(&self) -> usize {
        self.lines[self.at]
    }

    fn step(&mut self, forward: bool) {
        let count = self.lines.len();
        self.at = (self.at + if forward { 1 } else { count - 1 }) % count;
    }
}

struct Book<'a> {
    lines: Vec<&'a str>,
    plain: Vec<String>,
    marks: &'a [Mark],
}

impl<'a> Book<'a> {
    fn new(text: &'a str, marks: &'a [Mark]) -> Book<'a> {
        let lines: Vec<&str> = text.trim_end_matches('\n').lines().collect();
        Book {
            plain: lines.iter().map(|line| bare(line)).collect(),
            lines,
            marks,
        }
    }

    fn starts(&self) -> Vec<usize> {
        let mut starts = vec![0];
        starts.extend(
            self.marks
                .iter()
                .map(|mark| mark.line)
                .filter(|line| *line > 0 && *line < self.lines.len()),
        );
        starts
    }

    fn end(&self, starts: &[usize], index: usize) -> usize {
        let mut end = starts.get(index + 1).copied().unwrap_or(self.lines.len());
        while end > starts[index] && self.lines[end - 1].is_empty() {
            end -= 1;
        }
        end
    }

    fn pages(&self, view: usize) -> Vec<usize> {
        let starts = self.starts();
        let mut tops = vec![0];
        let mut top = 0;
        while top + view < self.lines.len() {
            let bottom = top + view;
            let cut = (0..starts.len()).find(|index| self.end(&starts, *index) > bottom);
            top = match cut {
                Some(index) if starts[index] > top => starts[index],
                _ => bottom,
            };
            tops.push(top);
        }
        if let [_, .., last] = tops.as_mut_slice() {
            *last = self.lines.len().saturating_sub(view);
        }
        tops
    }

    fn title(&self, top: usize) -> &str {
        if top == 0 {
            return "top";
        }
        self.marks
            .iter()
            .rfind(|mark| mark.line <= top)
            .map_or("top", |mark| mark.title.as_str())
    }

    fn ahead(&self, start: usize, seen: usize) -> &str {
        let fresh = self.marks.iter().find(|mark| mark.line >= seen);
        match fresh {
            Some(mark) if start < seen => mark.title.as_str(),
            _ => self.title(start),
        }
    }

    fn spans(&self, line: usize, query: &str) -> Vec<Range<usize>> {
        spans(&self.plain[line], query)
    }

    fn search(&self, query: &str, from: usize) -> Option<Search> {
        let lines: Vec<usize> = (0..self.lines.len())
            .filter(|line| !self.spans(*line, query).is_empty())
            .collect();
        let at = lines.iter().position(|line| *line >= from).unwrap_or(0);
        (!lines.is_empty()).then(|| Search {
            query: query.to_string(),
            lines,
            at,
        })
    }

    fn place(&self, line: usize, view: usize) -> usize {
        let start = self
            .marks
            .iter()
            .rfind(|mark| mark.line <= line)
            .map_or(0, |mark| mark.line);
        let top = if line - start < view {
            start
        } else {
            line.saturating_sub(CONTEXT)
        };
        top.min(self.lines.len().saturating_sub(view))
    }

    fn sections(&self) -> Vec<(usize, &str)> {
        let mut sections = vec![(0, "top")];
        sections.extend(
            self.marks
                .iter()
                .filter(|mark| mark.line > 0)
                .map(|mark| (mark.line, mark.title.as_str())),
        );
        sections.truncate(LETTERS.len());
        sections
    }
}

pub fn spans(plain: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    let exact = query.chars().any(char::is_uppercase);
    let text = if exact {
        plain.to_string()
    } else {
        plain.to_ascii_lowercase()
    };
    text.match_indices(query)
        .map(|(at, found)| {
            let start = text[..at].chars().count();
            start..start + found.chars().count()
        })
        .collect()
}

pub fn put(out: &mut String, style: &str, text: &str, styled: bool) {
    if styled {
        out.push_str(style);
        out.push_str(text);
        out.push_str(RESET);
    } else {
        out.push_str(text);
    }
}

pub struct Hint<'a> {
    pub rank: u8,
    pub letter: &'a str,
    pub word: String,
}

pub fn hint<'a>(rank: u8, letter: &'a str, word: &str) -> Hint<'a> {
    Hint {
        rank,
        letter,
        word: word.to_string(),
    }
}

pub fn bar(lead: String, mut hints: Vec<Hint<'_>>, columns: usize, styled: bool) -> String {
    let width = |hints: &[Hint<'_>]| {
        let hints: usize = hints
            .iter()
            .map(|hint| SPACE.len() + hint.letter.len() + 1 + hint.word.chars().count())
            .sum();
        bare(&lead).chars().count() + hints
    };
    while width(&hints) > columns {
        let Some(least) = hints
            .iter()
            .map(|hint| hint.rank)
            .max()
            .filter(|rank| *rank > 0)
        else {
            break;
        };
        hints.retain(|hint| hint.rank != least);
    }
    let mut out = lead;
    for hint in &hints {
        out.push_str(SPACE);
        put(&mut out, BOLD, hint.letter, styled);
        put(&mut out, DIM, &format!(" {}", hint.word), styled);
    }
    out
}

fn chip(text: &str, styled: bool) -> String {
    let mut out = String::from(MARGIN);
    put(&mut out, PLACE, text, styled);
    out
}

fn footer(book: &Book<'_>, pages: &[usize], rows: &Range<usize>, face: Bar) -> String {
    let top = rows.start;
    let page = pages.iter().rposition(|start| *start <= top).unwrap_or(0);
    let lead = chip(&format!("{}/{}", page + 1, pages.len()), face.styled);
    let ahead = pages.iter().find(|start| **start > top).map(|start| {
        if pages.last() == Some(start) {
            book.ahead(*start, rows.end)
        } else {
            book.title(*start)
        }
    });
    let behind = pages
        .iter()
        .rfind(|start| **start < top)
        .map(|start| book.title(*start));
    let mut hints = Vec::with_capacity(8);
    let plain = book.marks.is_empty();
    let named = |word: &str, title: &str| match plain {
        true => word.to_string(),
        false => format!("{word}: {title}"),
    };
    if let Some(title) = ahead {
        hints.push(hint(0, "n", &named("next", title)));
    }
    if let Some(title) = behind {
        hints.push(hint(0, "b", &named("back", title)));
    }
    hints.push(hint(2, "/", "search"));
    if !plain {
        hints.push(hint(3, "s", "sections"));
    }
    if face.flags {
        hints.push(hint(4, "-", "flag"));
    }
    hints.push(hint(1, "?", "keys"));
    if face.nested {
        hints.push(hint(0, "Esc", "close"));
    }
    hints.push(hint(0, "q", "quit"));
    bar(lead, hints, face.columns, face.styled)
}

fn found(search: &Search, face: Bar) -> String {
    let mut lead = String::from(MARGIN);
    put(&mut lead, BOLD, &format!("/{}", search.query), face.styled);
    lead.push_str(SPACE);
    put(
        &mut lead,
        PLACE,
        &format!("{}/{}", search.at + 1, search.lines.len()),
        face.styled,
    );
    let hints = vec![
        hint(0, "n", "next match"),
        hint(0, "b", "previous"),
        hint(0, "Esc", "clear"),
        hint(0, "q", "quit"),
    ];
    bar(lead, hints, face.columns, face.styled)
}

pub fn field(sign: &str, query: &str, styled: bool) -> String {
    let mut out = String::from(MARGIN);
    put(&mut out, BOLD, &format!("{sign}{query}"), styled);
    out.push_str(HIT);
    out.push(' ');
    out.push_str(HIT_END);
    out.push_str(SPACE);
    out
}

fn prompt(query: &str, search: Option<&Search>, face: Bar) -> String {
    let mut lead = field("/", query, face.styled);
    match search {
        Some(search) if search.lines.len() == 1 => lead.push_str("on 1 line"),
        Some(search) => lead.push_str(&format!("on {} lines", search.lines.len())),
        None if query.is_empty() => put(&mut lead, DIM, "type what to find", face.styled),
        None => lead.push_str("not found"),
    }
    let mut hints = Vec::with_capacity(2);
    if search.is_some() {
        hints.push(hint(0, "Enter", "keep"));
    }
    hints.push(hint(0, "Esc", "cancel"));
    bar(lead, hints, face.columns, face.styled)
}

pub fn marked(line: &str, spans: &[Range<usize>], on: &str, off: &str) -> String {
    let mut out = String::with_capacity(line.len() + 32);
    let mut styling = false;
    let mut inside = false;
    let mut column = 0;
    for letter in line.chars() {
        if styling {
            out.push(letter);
            styling = letter != 'm';
            if !styling && inside {
                out.push_str(on);
            }
            continue;
        }
        if letter == '\x1b' {
            styling = true;
            out.push(letter);
            continue;
        }
        let hit = spans.iter().any(|span| span.contains(&column));
        if hit != inside {
            out.push_str(if hit { on } else { off });
            inside = hit;
        }
        out.push(letter);
        column += 1;
    }
    if inside {
        out.push_str(off);
    }
    out
}

pub fn paint(tty: &mut File, rows: &[String], view: usize, footer: &str) -> io::Result<()> {
    let mut frame = String::with_capacity(8192);
    frame.push_str(HOME);
    for row in 0..view {
        if let Some(line) = rows.get(row) {
            frame.push_str(line);
        }
        frame.push_str(CLEAR_LINE);
        frame.push_str("\r\n");
    }
    frame.push_str(footer);
    frame.push_str(CLEAR_LINE);
    tty.write_all(frame.as_bytes())?;
    tty.flush()
}

fn lit(book: &Book<'_>, rows: Range<usize>, search: Option<&Search>) -> Vec<String> {
    rows.map(|line| {
        let spans = search.map_or_else(Vec::new, |search| book.spans(line, &search.query));
        match search {
            Some(search) if !spans.is_empty() && search.line() == line => {
                marked(book.lines[line], &spans, CHOSEN, CHOSEN_END)
            }
            Some(_) if !spans.is_empty() => marked(book.lines[line], &spans, HIT, HIT_END),
            _ => book.lines[line].to_string(),
        }
    })
    .collect()
}

fn board(title: &str, rows: &[(String, String)], styled: bool) -> Vec<String> {
    let column = rows
        .iter()
        .map(|(left, _)| left.chars().count())
        .max()
        .unwrap_or(0);
    let mut lines = vec![String::new()];
    let mut heading = String::from(MARGIN);
    put(&mut heading, BOLD, title, styled);
    lines.push(heading);
    lines.push(String::new());
    for (left, right) in rows {
        let mut line = String::from(INDENT);
        put(&mut line, PLACE, left, styled);
        line.push_str(&" ".repeat(column - left.chars().count()));
        line.push_str(SPACE);
        line.push_str(right);
        lines.push(line);
    }
    lines
}

fn window(tty: &File) -> (Size, usize) {
    let shown = size(tty).unwrap_or(FALLBACK);
    (shown, shown.rows.saturating_sub(1).max(1))
}

fn keys(tty: &mut File, face: Bar) -> io::Result<()> {
    let rows: Vec<(String, String)> = KEYS
        .iter()
        .map(|(left, right)| (left.to_string(), right.to_string()))
        .collect();
    loop {
        let (shown, view) = window(tty);
        let lines = board("KEYS", &rows, face.styled);
        let line = bar(
            String::from(MARGIN),
            vec![hint(0, "any key", "back")],
            shown.columns,
            face.styled,
        );
        paint(tty, &lines, view, &line)?;
        if !resized(tty, shown) {
            letter(tty)?;
            return Ok(());
        }
    }
}

fn sections(book: &Book<'_>, tty: &mut File, face: Bar) -> io::Result<Option<usize>> {
    let sections = book.sections();
    let rows: Vec<(String, String)> = sections
        .iter()
        .zip(LETTERS.chars())
        .map(|((_, title), letter)| (letter.to_string(), title.to_string()))
        .collect();
    loop {
        let (shown, view) = window(tty);
        let lines = board("SECTIONS", &rows, face.styled);
        let hints = vec![hint(0, "a letter", "jump there"), hint(0, "Esc", "back")];
        let line = bar(String::from(MARGIN), hints, shown.columns, face.styled);
        paint(tty, &lines, view, &line)?;
        if resized(tty, shown) {
            continue;
        }
        let chosen = letter(tty)?.and_then(|letter| LETTERS.find(letter));
        return Ok(chosen
            .and_then(|index| sections.get(index))
            .map(|(line, _)| *line));
    }
}

fn ask(book: &Book<'_>, tty: &mut File, origin: usize, face: Bar) -> io::Result<Option<Search>> {
    let mut draft: Vec<u8> = Vec::new();
    loop {
        let (shown, view) = window(tty);
        let query = String::from_utf8_lossy(&draft).into_owned();
        let search = book.search(&query, origin);
        let top = match &search {
            Some(search) => book.place(search.line(), view),
            None => origin.min(book.lines.len().saturating_sub(view)),
        };
        let rows = top..(top + view).min(book.lines.len());
        let face = Bar {
            columns: shown.columns,
            ..face
        };
        let line = prompt(&query, search.as_ref(), face);
        paint(tty, &lit(book, rows, search.as_ref()), view, &line)?;
        if resized(tty, shown) {
            continue;
        }
        match typed(tty)? {
            Typed::Byte(letter) => draft.push(letter),
            Typed::Erase if draft.is_empty() => return Ok(None),
            Typed::Erase => erase(&mut draft),
            Typed::Wipe => draft.clear(),
            Typed::Submit => return Ok(search),
            Typed::Cancel => return Ok(None),
            Typed::Other => {}
        }
    }
}

enum Leave {
    Quit(String),
    Open(Sheet, usize),
    Return,
}

fn flag(tty: &mut File, setup: &Setup<'_>, face: Bar) -> io::Result<Result<Sheet, String>> {
    let mut lead = field("-", "", face.styled);
    lead.push_str("the letter of a flag, such as n for --name");
    let line = bar(
        lead,
        vec![hint(0, "Esc", "cancel")],
        face.columns,
        face.styled,
    );
    tty.write_all(format!("\r{line}{CLEAR_LINE}").as_bytes())?;
    tty.flush()?;
    let Some(short) = letter(tty)? else {
        return Ok(Err(String::new()));
    };
    let sheet = setup.open.and_then(|open| open(short));
    Ok(sheet.ok_or_else(|| format!("There is no flag -{short}.")))
}

fn browse(
    book: &Book<'_>,
    tty: &mut File,
    setup: &Setup<'_>,
    mut top: usize,
    nested: bool,
) -> io::Result<Leave> {
    let mut search: Option<Search> = None;
    let mut notice = String::new();
    loop {
        let (shown, view) = window(tty);
        let pages = book.pages(view);
        let last = pages.last().copied().unwrap_or(0);
        top = top.min(last);
        let rows = top..(top + view).min(book.lines.len());
        let face = Bar {
            columns: shown.columns,
            styled: setup.styled,
            nested,
            flags: setup.open.is_some(),
        };
        let line = match &search {
            _ if !notice.is_empty() => format!("{MARGIN}{}", std::mem::take(&mut notice)),
            Some(search) => found(search, face),
            None => footer(book, &pages, &rows, face),
        };
        paint(tty, &lit(book, rows.clone(), search.as_ref()), view, &line)?;
        if resized(tty, shown) {
            continue;
        }
        let ahead = pages.iter().find(|start| **start > top);
        let behind = pages.iter().rfind(|start| **start < top);
        match (key(tty)?, &mut search) {
            (Key::Search, _) => {
                search = ask(book, tty, top, face)?;
                if let Some(search) = &search {
                    top = book.place(search.line(), view);
                }
            }
            (Key::Next, Some(search)) => {
                search.step(true);
                top = book.place(search.line(), view);
            }
            (Key::Back, Some(search)) => {
                search.step(false);
                top = book.place(search.line(), view);
            }
            (Key::Escape, Some(_)) => search = None,
            (Key::Escape, None) if nested => return Ok(Leave::Return),
            (Key::Sections, _) => top = sections(book, tty, face)?.unwrap_or(top),
            (Key::Keys, _) => keys(tty, face)?,
            (Key::Flag, _) if setup.open.is_some() => match flag(tty, setup, face)? {
                Ok(sheet) => return Ok(Leave::Open(sheet, top)),
                Err(message) => notice = message,
            },
            (Key::Next | Key::PageDown, _) => top = ahead.map_or(top, |start| *start),
            (Key::Back | Key::PageUp, _) => top = behind.map_or(top, |start| *start),
            (Key::Down | Key::Enter, _) => top = (top + 1).min(last),
            (Key::Up, _) => top = top.saturating_sub(1),
            (Key::First, _) => top = 0,
            (Key::Last, _) => top = last,
            (Key::Quit | Key::Escape, _) => return Ok(Leave::Quit(leave(book, rows))),
            (Key::Flag | Key::Other, _) => {}
        }
    }
}

fn leave(book: &Book<'_>, rows: Range<usize>) -> String {
    let rows = &book.lines[rows];
    let mut out = String::with_capacity(4096);
    if rows.first().is_some_and(|line| !line.is_empty()) {
        out.push('\n');
    }
    for line in rows {
        out.push_str(line);
        out.push('\n');
    }
    if rows.last().is_some_and(|line| !line.is_empty()) {
        out.push('\n');
    }
    out
}

pub fn offer(sheet: Sheet, setup: &Setup<'_>) -> Result<(), Sheet> {
    if !needed(&sheet.text) {
        return Err(sheet);
    }
    show(sheet, setup)
}

fn show(sheet: Sheet, setup: &Setup<'_>) -> Result<(), Sheet> {
    let Ok(mut raw) = Raw::enter() else {
        return Err(sheet);
    };
    let _ = raw.tty.write_all(ENTER.as_bytes());
    let mut stack: Vec<(Sheet, usize)> = Vec::new();
    let mut current = sheet;
    let mut top = 0;
    let left = loop {
        let outcome = {
            let book = Book::new(&current.text, &current.marks);
            browse(&book, &mut raw.tty, setup, top, !stack.is_empty())
        };
        match (outcome, stack.pop()) {
            (Ok(Leave::Open(next, at)), before) => {
                stack.extend(before);
                stack.push((std::mem::replace(&mut current, next), at));
                top = 0;
            }
            (Ok(Leave::Return), Some((before, at))) => {
                current = before;
                top = at;
            }
            (Ok(Leave::Quit(text)), _) => break text,
            (Ok(Leave::Return), None) | (Err(_), _) => break String::new(),
        }
    };
    let _ = raw.tty.write_all(LEAVE.as_bytes());
    drop(raw);
    let mut out = io::stdout().lock();
    let _ = out.write_all(left.as_bytes());
    let _ = out.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDE: Bar = Bar {
        columns: 200,
        styled: false,
        nested: false,
        flags: false,
    };

    fn sample() -> (String, Vec<Mark>) {
        let mut text = String::new();
        let mut marks = Vec::new();
        for (title, rows) in [("ONE", 3), ("TWO", 4), ("THREE", 12), ("FOUR", 2)] {
            text.push('\n');
            marks.push(Mark {
                line: text.matches('\n').count(),
                title: title.to_string(),
            });
            text.push_str(title);
            text.push('\n');
            for row in 0..rows {
                text.push_str(&format!("{title} {row}\n"));
            }
        }
        text.push('\n');
        (text, marks)
    }

    #[test]
    fn screens_start_at_a_heading_and_the_last_one_is_full() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let pages = book.pages(14);
        assert_eq!(pages, [0, 12, 15]);
        assert_eq!(book.lines[12], "THREE");
        assert_eq!(book.lines.len(), 15 + 14);
        assert_eq!(book.title(0), "top");
        assert_eq!(book.title(12), "THREE");
        assert_eq!(book.title(13), "THREE");
    }

    #[test]
    fn a_section_taller_than_the_screen_is_paged_through() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let pages = book.pages(6);
        assert_eq!(pages, [0, 6, 12, 18, 23]);
        assert_eq!(book.title(18), "THREE");
        let lines = book.lines.len();
        assert!(pages.windows(2).all(|pair| pair[1] - pair[0] <= 6));
        assert_eq!(pages[pages.len() - 1] + 6, lines);
    }

    #[test]
    fn a_page_that_fits_is_one_screen() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        assert_eq!(book.pages(book.lines.len()), [0]);
    }

    #[test]
    fn the_last_screen_names_the_first_unseen_section() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        assert_eq!(book.pages(24), [0, 5]);
        assert_eq!(
            footer(&book, &[0, 5], &(0..24), WIDE),
            "  1/2   n next: FOUR   / search   s sections   ? keys   q quit"
        );
    }

    #[test]
    fn a_page_without_headings_gets_a_plain_footer() {
        let (text, _) = sample();
        let book = Book::new(&text, &[]);
        let pages = book.pages(14);
        assert_eq!(pages, [0, 14, 15]);
        assert_eq!(
            footer(&book, &pages, &(14..28), WIDE),
            "  2/3   n next   b back   / search   ? keys   q quit"
        );
    }

    #[test]
    fn hints_are_dropped_when_the_window_is_narrow() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let pages = book.pages(14);
        let narrow = Bar {
            columns: 44,
            flags: true,
            ..WIDE
        };
        assert_eq!(
            footer(&book, &pages, &(12..26), narrow),
            "  2/3   n next: FOUR   b back: top   q quit"
        );
        let wider = Bar {
            columns: 64,
            ..narrow
        };
        assert_eq!(
            footer(&book, &pages, &(12..26), wider),
            "  2/3   n next: FOUR   b back: top   / search   ? keys   q quit"
        );
    }

    #[test]
    fn styling_does_not_count_as_width() {
        assert_eq!(bare("\x1b[1mUSAGE\x1b[0m \x1b[2m──\x1b[0m"), "USAGE ──");
        assert_eq!(bare("plain"), "plain");
    }

    #[test]
    fn the_footer_names_where_the_keys_lead() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let pages = book.pages(14);
        assert_eq!(
            footer(&book, &pages, &(0..14), WIDE),
            "  1/3   n next: THREE   / search   s sections   ? keys   q quit"
        );
        assert_eq!(
            footer(&book, &pages, &(12..26), WIDE),
            "  2/3   n next: FOUR   b back: top   / search   s sections   ? keys   q quit"
        );
        assert_eq!(
            footer(&book, &pages, &(15..29), WIDE),
            "  3/3   b back: THREE   / search   s sections   ? keys   q quit"
        );
        assert!(
            footer(
                &book,
                &pages,
                &(12..26),
                Bar {
                    styled: true,
                    ..WIDE
                }
            )
            .contains(&format!("{BOLD}n{RESET}"))
        );
    }

    #[test]
    fn the_screen_in_view_is_left_behind() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        assert_eq!(leave(&book, 26..29), "\nFOUR\nFOUR 0\nFOUR 1\n\n");
        assert!(leave(&book, 0..5).starts_with("\nONE\n"));
        assert!(leave(&book, 0..5).ends_with("ONE 2\n\n"));
    }

    #[test]
    fn keys_are_read_from_escape_sequences() {
        assert_eq!(named(b"[A"), Key::Up);
        assert_eq!(named(b"OB"), Key::Down);
        assert_eq!(named(b"[6~"), Key::PageDown);
        assert_eq!(named(b"[5~"), Key::PageUp);
        assert_eq!(named(b"[H"), Key::First);
        assert_eq!(named(b"[F"), Key::Last);
        assert_eq!(named(b"[Z"), Key::Other);
    }

    #[test]
    fn a_search_finds_lines_and_starts_at_the_screen_in_view() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let Some(mut search) = book.search("two", 0) else {
            panic!("two is in the text");
        };
        assert_eq!(search.lines, [6, 7, 8, 9, 10]);
        assert_eq!(search.line(), 6);
        search.step(false);
        assert_eq!(search.line(), 10);
        search.step(true);
        assert_eq!(search.line(), 6);
        assert_eq!(book.search("1", 12).map(|found| found.line()), Some(14));
        assert_eq!(book.search("ONE 1", 12).map(|found| found.line()), Some(3));
        assert!(book.search("Two", 0).is_none());
        assert!(book.search("", 0).is_none());
    }

    #[test]
    fn a_match_is_shown_below_its_heading() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        assert_eq!(book.place(8, 14), 6);
        assert_eq!(book.place(24, 14), 12);
        assert_eq!(book.place(24, 6), 22);
        assert_eq!(book.place(28, 14), 15);
    }

    fn span(start: usize, end: usize) -> Range<usize> {
        Range { start, end }
    }

    #[test]
    fn matches_are_marked_through_the_styling() {
        let text = "\n\x1b[1mUSAGE\x1b[0m \x1b[2m──\x1b[0m\n  \x1b[36m-n\x1b[0m\x1b[2m, \x1b[0m\x1b[36m--name\x1b[0m\n";
        let book = Book::new(text, &[]);
        assert_eq!(book.spans(1, "sag"), [span(1, 4)]);
        assert_eq!(book.spans(1, "SAG"), [span(1, 4)]);
        assert!(book.spans(1, "Sag").is_empty());
        assert_eq!(book.spans(2, "n"), [3..4, 8..9]);
        let spans = book.spans(2, "n, --na");
        assert_eq!(spans, [span(3, 10)]);
        assert_eq!(
            marked(book.lines[2], &spans, "<", ">"),
            "  \x1b[36m-<n\x1b[0m<\x1b[2m<, \x1b[0m<\x1b[36m<--na>me\x1b[0m"
        );
        assert_eq!(
            marked("plain text", &[span(6, 10)], "<", ">"),
            "plain <text>"
        );
    }

    #[test]
    fn the_bottom_line_follows_the_search() {
        let (text, marks) = sample();
        let book = Book::new(&text, &marks);
        let search = book.search("four", 0);
        assert_eq!(
            prompt("four", search.as_ref(), WIDE),
            format!("  /four{HIT} {HIT_END}   on 3 lines   Enter keep   Esc cancel")
        );
        assert_eq!(
            prompt("zz", None, WIDE),
            format!("  /zz{HIT} {HIT_END}   not found   Esc cancel")
        );
        let Some(search) = search else {
            panic!("four is in the text");
        };
        assert_eq!(
            found(&search, WIDE),
            "  /four   1/3   n next match   b previous   Esc clear   q quit"
        );
    }
}
