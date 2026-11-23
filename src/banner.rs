use std::io::{self, IsTerminal, Write};
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use crate::pager;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const BORDER: &str = "\x1b[38;5;60m";
const GREETING: &str = "\x1b[1;38;5;211m";
const ACCENT: &str = "\x1b[38;5;81m";
const HEADING: &str = "\x1b[1;38;5;177m";
const PATTERN: &str = "\x1b[1;38;5;204m";
const PATH: &str = "\x1b[1;38;5;135m";
const FILTER: &str = "\x1b[38;5;214m";
const FLAG: &str = "\x1b[38;5;39m";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";

const AURORA: [&str; 6] = [
    "\x1b[38;5;221m", // gold crest
    "\x1b[38;5;210m", // coral peach
    "\x1b[38;5;211m", // rose pink
    "\x1b[38;5;177m", // aurora orchid
    "\x1b[38;5;141m", // luminous violet
    "\x1b[38;5;81m",  // electric cyan
];

const MARGIN: &str = "  ";
const GAP: &str = "   ";
pub const WIDTH: usize = 80;
const BOX_MIN_COLUMNS: usize = 64;
const BOX_MAX_WIDTH: usize = 76;
const BLINK_DELAY: Duration = Duration::from_millis(180);
const BLINK_HOLD: Duration = Duration::from_millis(115);

const ART: [&str; 5] = [
    "    ▄▄████▀ ",
    " ▄████████▄ ",
    "▄████ ██ ██▄",
    "▀████▄██▄██▀",
    "  ▀▀████▀▀  ",
];
const BLINK_EYE_ROW: &str = "▄██████████▄";
const NAME_ROW: usize = 2;
const ABOUT_ROW: usize = 3;

pub const TAGLINE: &str = "Search folder names, file names and contents in one pass.";
pub const EXPLAIN: &str = "How these arguments are read.";
pub const PICKS: &str = "The paths you chose with -1, -b or -O.";
pub const SETUP: &str = "Tab completion and the jump command for your shell.";

static HIDDEN: AtomicBool = AtomicBool::new(false);

pub fn hide_logo() {
    HIDDEN.store(true, Ordering::Relaxed);
}

#[derive(Clone, Copy)]
pub struct Look {
    pub banner: bool,
    pub logo: bool,
    pub styled: bool,
    pub columns: usize,
}

impl Look {
    pub fn of(stream: &(impl IsTerminal + AsFd)) -> Look {
        let terminal = stream.is_terminal();
        Look {
            banner: terminal,
            logo: terminal && !HIDDEN.load(Ordering::Relaxed),
            styled: terminal && std::env::var_os("NO_COLOR").is_none(),
            columns: pager::size(stream).map_or(WIDTH, |size| size.columns.min(WIDTH)),
        }
    }
}

fn put(out: &mut String, style: &str, text: &str, styled: bool) {
    if styled {
        out.push_str(style);
        out.push_str(text);
        out.push_str(RESET);
    } else {
        out.push_str(text);
    }
}

fn tone(row: usize, col: usize) -> &'static str {
    let step = row * 3 + col / 2;
    let band = match step {
        0..=4 => 0,
        5..=7 => 1,
        8..=10 => 2,
        11..=12 => 3,
        13..=14 => 4,
        _ => 5,
    };
    AURORA[band]
}

fn paint_art(out: &mut String, row: usize, art: &str, styled: bool) {
    if !styled {
        out.push_str(art);
        return;
    }
    let mut current: Option<&str> = None;
    for (col, ch) in art.chars().enumerate() {
        if ch == ' ' {
            if current.take().is_some() {
                out.push_str(RESET);
            }
            out.push(' ');
            continue;
        }
        let color = tone(row, col);
        if current != Some(color) {
            out.push_str(color);
            current = Some(color);
        }
        out.push(ch);
    }
    if current.is_some() {
        out.push_str(RESET);
    }
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    if max <= 3 {
        return text.chars().take(max).collect();
    }
    let mut short: String = text.chars().take(max - 3).collect();
    short.push_str("...");
    short
}

fn fitted(about: &str, columns: usize) -> String {
    let art_width = ART[ABOUT_ROW].chars().count();
    let room = columns.saturating_sub(MARGIN.len() + art_width + GAP.len());
    clip(about, room)
}

pub fn header(about: &str, look: Look) -> String {
    if !look.logo {
        return String::new();
    }
    let mut out = String::with_capacity(512);
    out.push('\n');
    for (row, art) in ART.iter().enumerate() {
        out.push_str(MARGIN);
        match row {
            NAME_ROW => {
                paint_art(&mut out, row, art, look.styled);
                out.push_str(GAP);
                put(&mut out, BOLD, "syk", look.styled);
                out.push(' ');
                put(&mut out, DIM, env!("CARGO_PKG_VERSION"), look.styled);
            }
            ABOUT_ROW => {
                paint_art(&mut out, row, art, look.styled);
                out.push_str(GAP);
                out.push_str(&fitted(about, look.columns));
            }
            _ => paint_art(&mut out, row, art.trim_end(), look.styled),
        }
        out.push('\n');
    }
    out
}

pub fn above(about: &str, look: Look) -> String {
    let mut out = header(about, look);
    if look.logo {
        out.push('\n');
    }
    out
}

struct Row {
    text: String,
    width: usize,
}

impl Row {
    fn new() -> Self {
        Self {
            text: String::with_capacity(128),
            width: 0,
        }
    }

    fn plain(&mut self, text: &str) {
        self.text.push_str(text);
        self.width += text.chars().count();
    }

    fn styled(&mut self, style: &str, text: &str, styled: bool) {
        put(&mut self.text, style, text, styled);
        self.width += text.chars().count();
    }

    fn art(&mut self, row: usize, art: &str, styled: bool) {
        paint_art(&mut self.text, row, art, styled);
        self.width += art.chars().count();
    }

    fn pad_to(&mut self, target: usize) {
        let extra = target.saturating_sub(self.width);
        for _ in 0..extra {
            self.text.push(' ');
        }
        self.width += extra;
    }
}

struct Eye {
    line: usize,
    col: usize,
}

struct Card {
    text: String,
    lines: usize,
    eye: Option<Eye>,
}

fn title_row(room: usize, styled: bool) -> Row {
    let version = env!("CARGO_PKG_VERSION");
    let full_len = "Wolkom! syk ".len() + version.len() + "  ·  ready to seek".chars().count();
    let short_len = "Wolkom! syk ".len() + version.len();

    let mut row = Row::new();
    if room >= short_len {
        row.styled(GREETING, "Wolkom!", styled);
        row.plain(" ");
        row.styled(BOLD, "syk", styled);
        row.plain(" ");
        row.styled(DIM, version, styled);
        if room >= full_len {
            row.styled(DIM, "  ·  ", styled);
            row.styled(ACCENT, "ready to seek", styled);
        }
    } else {
        let fallback = clip(&format!("Wolkom! syk {version}"), room);
        row.styled(BOLD, &fallback, styled);
    }
    row
}

fn status_row(status: &str, room: usize, styled: bool) -> Row {
    let mut row = Row::new();
    if room >= 3 {
        row.styled(ACCENT, "✓", styled);
        row.plain(" ");
        row.plain(&clip(status, room - 2));
    } else {
        row.plain(&clip(status, room));
    }
    row
}

fn example_rows(jump: Option<&str>, content_width: usize, styled: bool) -> [Row; 4] {
    let cmd_col = 20.min(content_width / 2);

    let finish = |mut row: Row, desc: &str| {
        if row.width < cmd_col {
            row.pad_to(cmd_col);
        } else if row.width + 2 < content_width {
            row.plain("  ");
        }
        let room = content_width.saturating_sub(row.width);
        if room > 0 {
            row.styled(DIM, &clip(desc, room), styled);
        }
        row
    };

    let mut r1 = Row::new();
    r1.styled(BOLD, "syk", styled);
    r1.plain(" ");
    r1.styled(PATTERN, "auth", styled);

    let mut r2 = Row::new();
    r2.styled(BOLD, "syk", styled);
    r2.plain(" ");
    r2.styled(PATTERN, "auth", styled);
    r2.plain(" ");
    r2.styled(FILTER, ".ts", styled);
    r2.plain(" ");
    r2.styled(PATH, "src/", styled);

    let mut r3 = Row::new();
    r3.styled(BOLD, "syk", styled);
    r3.plain(" ");
    r3.styled(PATTERN, "auth", styled);
    r3.plain(" ");
    r3.styled(FLAG, "-1", styled);

    let mut r4 = Row::new();
    let r4_desc = if let Some(func) = jump {
        let max_func = cmd_col.saturating_sub(" projects  ".len()).max(1);
        let name = clip(func, max_func);
        r4.styled(BOLD, &name, styled);
        r4.plain(" ");
        r4.styled(PATH, "projects", styled);
        "fuzzy cd via your new shell command"
    } else {
        r4.styled(BOLD, "syk", styled);
        r4.plain(" ");
        r4.styled(FLAG, "--setup", styled);
        "add shell completions & the s jump command"
    };

    [
        finish(r1, "search folders, files & contents in one pass"),
        finish(r2, "narrow by extension & folder"),
        finish(r3, "jump straight to the best match"),
        finish(r4, r4_desc),
    ]
}

fn footer_row(content_width: usize, styled: bool) -> Row {
    let full = "syk -h for flags  ·  syk foo -x to inspect a query";
    let mut row = Row::new();
    if full.chars().count() <= content_width {
        row.styled(BOLD, "syk", styled);
        row.plain(" ");
        row.styled(FLAG, "-h", styled);
        row.styled(DIM, " for flags  ·  ", styled);
        row.styled(BOLD, "syk", styled);
        row.plain(" ");
        row.styled(PATTERN, "foo", styled);
        row.plain(" ");
        row.styled(FLAG, "-x", styled);
        row.styled(DIM, " to inspect a query", styled);
    } else {
        row.styled(DIM, &clip(full, content_width), styled);
    }
    row
}

fn build_welcome(status: &str, jump: Option<&str>, look: Look) -> Card {
    let mut out = String::with_capacity(2048);
    let mut lines = 0usize;

    if look.logo && look.columns >= BOX_MIN_COLUMNS {
        let box_width = look.columns.saturating_sub(MARGIN.len()).min(BOX_MAX_WIDTH);
        let inner = box_width - 2;
        let content_width = inner - 4;
        let art_width = ART[NAME_ROW].chars().count();
        let art_room = content_width.saturating_sub(art_width + GAP.len());
        let rule = "─".repeat(inner);

        let push_box = |out: &mut String, lines: &mut usize, mut row: Row| {
            row.pad_to(content_width);
            out.push_str(MARGIN);
            put(out, BORDER, "│", look.styled);
            out.push_str("  ");
            out.push_str(&row.text);
            out.push_str("  ");
            put(out, BORDER, "│", look.styled);
            out.push('\n');
            *lines += 1;
        };

        out.push('\n');
        lines += 1;

        out.push_str(MARGIN);
        put(&mut out, BORDER, &format!("╭{rule}╮"), look.styled);
        out.push('\n');
        lines += 1;

        push_box(&mut out, &mut lines, Row::new());

        let mut eye = None;
        for (idx, art) in ART.iter().enumerate() {
            if idx == NAME_ROW {
                eye = Some(Eye {
                    line: lines,
                    col: MARGIN.len() + 3,
                });
            }
            let mut row = Row::new();
            row.art(idx, art, look.styled);
            if idx == NAME_ROW {
                row.plain(GAP);
                let title = title_row(art_room, look.styled);
                row.text.push_str(&title.text);
                row.width += title.width;
            } else if idx == ABOUT_ROW {
                row.plain(GAP);
                let st = status_row(status, art_room, look.styled);
                row.text.push_str(&st.text);
                row.width += st.width;
            }
            push_box(&mut out, &mut lines, row);
        }

        push_box(&mut out, &mut lines, Row::new());

        let mut heading = Row::new();
        heading.styled(HEADING, "TRY FIRST", look.styled);
        push_box(&mut out, &mut lines, heading);

        for row in example_rows(jump, content_width, look.styled) {
            push_box(&mut out, &mut lines, row);
        }

        push_box(&mut out, &mut lines, Row::new());
        push_box(&mut out, &mut lines, footer_row(content_width, look.styled));

        out.push_str(MARGIN);
        put(&mut out, BORDER, &format!("╰{rule}╯"), look.styled);
        out.push('\n');
        lines += 1;

        return Card {
            text: out,
            lines,
            eye,
        };
    }

    let content_width = look.columns.saturating_sub(MARGIN.len()).max(20);
    let push_line = |out: &mut String, lines: &mut usize, row: Row| {
        out.push_str(MARGIN);
        out.push_str(&row.text);
        out.push('\n');
        *lines += 1;
    };

    out.push('\n');
    lines += 1;

    let mut eye = None;
    if look.logo {
        let art_width = ART[NAME_ROW].chars().count();
        let art_room = content_width.saturating_sub(art_width + GAP.len());
        for (idx, art) in ART.iter().enumerate() {
            if idx == NAME_ROW {
                eye = Some(Eye {
                    line: lines,
                    col: MARGIN.len(),
                });
            }
            let mut row = Row::new();
            match idx {
                NAME_ROW => {
                    row.art(idx, art, look.styled);
                    row.plain(GAP);
                    let title = title_row(art_room, look.styled);
                    row.text.push_str(&title.text);
                    row.width += title.width;
                }
                ABOUT_ROW => {
                    row.art(idx, art, look.styled);
                    row.plain(GAP);
                    let st = status_row(status, art_room, look.styled);
                    row.text.push_str(&st.text);
                    row.width += st.width;
                }
                _ => row.art(idx, art.trim_end(), look.styled),
            }
            push_line(&mut out, &mut lines, row);
        }
    } else {
        push_line(&mut out, &mut lines, title_row(content_width, look.styled));
        push_line(
            &mut out,
            &mut lines,
            status_row(status, content_width, look.styled),
        );
    }

    out.push('\n');
    lines += 1;

    let mut heading = Row::new();
    heading.styled(HEADING, "TRY FIRST", look.styled);
    push_line(&mut out, &mut lines, heading);

    for row in example_rows(jump, content_width, look.styled) {
        push_line(&mut out, &mut lines, row);
    }

    out.push('\n');
    lines += 1;

    push_line(&mut out, &mut lines, footer_row(content_width, look.styled));

    Card {
        text: out,
        lines,
        eye,
    }
}

#[cfg(test)]
fn welcome_card(status: &str, jump: Option<&str>, look: Look) -> String {
    build_welcome(status, jump, look).text
}

pub fn show_welcome(status: &str, jump: Option<&str>, look: Look) {
    let card = build_welcome(status, jump, look);
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(card.text.as_bytes());
    let _ = stdout.flush();

    let can_animate = look.banner
        && look.logo
        && look.styled
        && std::env::var("TERM").ok().as_deref() != Some("dumb")
        && pager::size(&stdout).is_some_and(|size| size.rows > card.lines);

    if !can_animate {
        return;
    }
    let Some(eye) = card.eye else {
        return;
    };

    let up = card.lines - eye.line;
    let mut closed = String::with_capacity(128);
    paint_art(&mut closed, NAME_ROW, BLINK_EYE_ROW, true);
    let mut open = String::with_capacity(128);
    paint_art(&mut open, NAME_ROW, ART[NAME_ROW], true);

    let _ = write!(stdout, "{HIDE_CURSOR}");
    let _ = stdout.flush();
    thread::sleep(BLINK_DELAY);

    let _ = write!(stdout, "\x1b[{up}A\r\x1b[{}C{closed}", eye.col);
    let _ = stdout.flush();
    thread::sleep(BLINK_HOLD);

    let _ = write!(stdout, "\r\x1b[{}C{open}\r\x1b[{up}B{SHOW_CURSOR}", eye.col);
    let _ = stdout.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: Look = Look {
        banner: true,
        logo: true,
        styled: false,
        columns: WIDTH,
    };

    fn strip_ansi(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars();
        while let Some(ch) = chars.next() {
            if ch == '\x1b' {
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            } else {
                out.push(ch);
            }
        }
        out
    }

    #[test]
    fn nothing_is_added_outside_a_terminal() {
        let piped = Look {
            banner: false,
            logo: false,
            styled: false,
            columns: WIDTH,
        };
        assert_eq!(header(TAGLINE, piped), "");
        assert_eq!(above(EXPLAIN, piped), "");
    }

    #[test]
    fn every_view_fits_the_width() {
        for about in [TAGLINE, EXPLAIN, PICKS, SETUP] {
            let text = header(about, SHOWN);
            assert!(!text.contains('\x1b'));
            assert!(text.contains(about), "{about:?} is cut short");
            assert!(text.lines().all(|line| line.chars().count() <= WIDTH));
            assert!(text.lines().all(|line| line == line.trim_end()));
        }
    }

    #[test]
    fn a_long_line_is_cut_to_fit() {
        let text = header(&"word ".repeat(40), SHOWN);
        assert!(text.contains("..."));
        assert!(text.lines().all(|line| line.chars().count() <= WIDTH));
    }

    #[test]
    fn the_logo_and_the_name_are_coloured_in_a_terminal() {
        let look = Look {
            banner: true,
            logo: true,
            styled: true,
            columns: WIDTH,
        };
        let text = header(TAGLINE, look);
        assert!(text.contains(&format!("{BOLD}syk{RESET}")));
        assert!(text.contains(&format!("{DIM}{}{RESET}", env!("CARGO_PKG_VERSION"))));
        for color in AURORA {
            assert!(text.contains(color), "missing aurora band {color:?}");
        }
        assert!(above(TAGLINE, look).ends_with("\n\n"));
    }

    #[test]
    fn welcome_card_aligns_every_boxed_row_and_fits_80_columns() {
        let look = Look {
            banner: true,
            logo: true,
            styled: true,
            columns: WIDTH,
        };
        let card = build_welcome("fish completions & j active in new shells", Some("j"), look);
        assert!(card.eye.is_some());
        assert_eq!(card.lines, card.text.lines().count());

        let plain = strip_ansi(&card.text);
        let box_lines: Vec<&str> = plain.lines().filter(|line| !line.is_empty()).collect();
        let expected_width = box_lines[0].chars().count();
        assert!(expected_width <= WIDTH);
        assert!(box_lines[0].starts_with("  ╭") && box_lines[0].ends_with('╮'));
        assert!(
            box_lines
                .last()
                .is_some_and(|line| line.starts_with("  ╰") && line.ends_with('╯'))
        );
        for line in &box_lines {
            assert_eq!(
                line.chars().count(),
                expected_width,
                "misaligned line: {line:?}"
            );
        }
        assert!(plain.contains("Wolkom! syk"));
        assert!(plain.contains("j projects"));
        for color in AURORA {
            assert!(card.text.contains(color), "missing aurora band {color:?}");
        }
    }

    #[test]
    fn welcome_card_stays_plain_when_unstyled_and_fits_narrow_terminals() {
        let narrow = Look {
            banner: true,
            logo: true,
            styled: false,
            columns: 48,
        };
        let text = welcome_card(
            "installed — run syk --setup for completions & s",
            None,
            narrow,
        );
        assert!(!text.contains('\x1b'));
        assert!(!text.contains('╭'));
        assert!(text.contains("syk --setup"));
        assert!(
            text.lines().all(|line| line.chars().count() <= 48),
            "narrow line overflow:\n{text}"
        );

        let piped = Look {
            banner: false,
            logo: false,
            styled: false,
            columns: WIDTH,
        };
        let plain = welcome_card("installed — run syk --setup", None, piped);
        assert!(!plain.contains('\x1b'));
        assert!(!plain.contains("▄▄████▀"));
        assert!(plain.contains("Wolkom! syk"));
        assert!(plain.contains("TRY FIRST"));
    }

    #[test]
    fn blink_row_matches_open_eye_row_width() {
        assert_eq!(BLINK_EYE_ROW.chars().count(), ART[NAME_ROW].chars().count());
    }
}
