use std::fs::File;
use std::io::{self, Write};

use crate::pager::{
    self, BOLD, DIM, ENTER, FALLBACK, HIT, HIT_END, Key, LEAVE, MARGIN, PLACE, RESET, Raw, Typed,
};
use crate::rank::Block;

const NUMBER: &str = "\x1b[38;5;39m";
const POINTER: &str = "> ";

pub struct Item {
    pub path: Vec<u8>,
    pub line: Option<u64>,
    plain: String,
    shown: String,
}

fn clean(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .chars()
        .map(|letter| if letter.is_control() { ' ' } else { letter })
        .collect()
}

fn item(block: &Block, row: &[u8], styled: bool) -> Item {
    let hit = row
        .strip_prefix(block.path.as_slice())
        .and_then(|rest| rest.strip_prefix(b":"))
        .and_then(|rest| {
            let digits = rest.iter().take_while(|byte| byte.is_ascii_digit()).count();
            let number = std::str::from_utf8(&rest[..digits])
                .ok()?
                .parse::<u64>()
                .ok()?;
            Some((number, rest[digits..].strip_prefix(b":")?))
        });
    let Some((number, text)) = hit else {
        let plain = clean(row);
        let mut shown = String::new();
        pager::put(&mut shown, BOLD, &plain, styled);
        return Item {
            path: block.path.clone(),
            line: None,
            plain,
            shown,
        };
    };
    let path = clean(&block.path);
    let text = clean(text);
    let text = text.trim();
    let mut shown = String::new();
    pager::put(&mut shown, BOLD, &path, styled);
    pager::put(&mut shown, NUMBER, &format!(":{number}"), styled);
    shown.push_str("  ");
    shown.push_str(text);
    Item {
        path: block.path.clone(),
        line: Some(number),
        plain: format!("{path}:{number}  {text}"),
        shown,
    }
}

pub fn items<'a>(blocks: impl Iterator<Item = &'a Block>, styled: bool) -> Vec<Item> {
    blocks
        .flat_map(|block| {
            block
                .text
                .split(|byte| *byte == b'\n')
                .filter(|row| !row.is_empty())
                .map(move |row| item(block, row, styled))
        })
        .collect()
}

fn narrowed(items: &[Item], filter: &str) -> Vec<usize> {
    let words: Vec<&str> = filter.split_whitespace().collect();
    (0..items.len())
        .filter(|index| {
            words
                .iter()
                .all(|word| !pager::spans(&items[*index].plain, word).is_empty())
        })
        .collect()
}

fn rows(items: &[Item], kept: &[usize], from: usize, view: usize, cursor: usize) -> Vec<String> {
    kept.iter()
        .enumerate()
        .skip(from)
        .take(view)
        .map(|(position, index)| {
            let item = &items[*index];
            if position == cursor {
                format!("{HIT}{POINTER}{}{HIT_END}", item.plain)
            } else {
                format!("{MARGIN}{}", item.shown)
            }
        })
        .collect()
}

fn footer(
    filter: &str,
    typing: bool,
    cursor: usize,
    kept: usize,
    all: usize,
    look: Look,
) -> String {
    let styled = look.styled;
    if typing {
        let mut lead = pager::field("/", filter, styled);
        lead.push_str(&format!("{kept} of {all}"));
        let hints = vec![
            pager::hint(0, "Enter", "keep"),
            pager::hint(0, "Esc", "clear"),
        ];
        return pager::bar(lead, hints, look.columns, styled);
    }
    let mut lead = String::from(MARGIN);
    let place = if kept == 0 { 0 } else { cursor + 1 };
    pager::put(&mut lead, PLACE, &format!("{place}/{kept}"), styled);
    if !filter.is_empty() {
        lead.push_str("   ");
        pager::put(&mut lead, BOLD, &format!("/{filter}"), styled);
    }
    let mut hints = vec![
        pager::hint(0, "Enter", "choose"),
        pager::hint(2, "j k", "move"),
        pager::hint(1, "/", "filter"),
    ];
    if !filter.is_empty() {
        hints.push(pager::hint(0, "Esc", "clear"));
    }
    hints.push(pager::hint(0, "q", "cancel"));
    pager::bar(lead, hints, look.columns, styled)
}

#[derive(Clone, Copy)]
struct Look {
    columns: usize,
    styled: bool,
}

fn empty(styled: bool) -> Vec<String> {
    let mut line = String::from(MARGIN);
    pager::put(&mut line, DIM, "Nothing matches the filter.", styled);
    vec![String::new(), line]
}

fn turn(items: &[Item], tty: &mut File, styled: bool) -> io::Result<Option<usize>> {
    let mut filter: Vec<u8> = Vec::new();
    let mut typing = false;
    let mut cursor = 0;
    let mut from = 0;
    loop {
        let shown = pager::size(tty).unwrap_or(FALLBACK);
        let view = shown.rows.saturating_sub(1).max(1);
        let text = String::from_utf8_lossy(&filter).into_owned();
        let kept = narrowed(items, &text);
        let last = kept.len().saturating_sub(1);
        cursor = cursor.min(last);
        from = from.min(cursor).max((cursor + 1).saturating_sub(view));
        let look = Look {
            columns: shown.columns,
            styled,
        };
        let line = footer(&text, typing, cursor, kept.len(), items.len(), look);
        let lines = match kept.is_empty() {
            true => empty(styled),
            false => rows(items, &kept, from, view, cursor),
        };
        pager::paint(tty, &lines, view, &line)?;
        if pager::resized(tty, shown) {
            continue;
        }
        if typing {
            match pager::typed(tty)? {
                Typed::Byte(letter) => {
                    filter.push(letter);
                    cursor = 0;
                }
                Typed::Erase if filter.is_empty() => typing = false,
                Typed::Erase => pager::erase(&mut filter),
                Typed::Wipe => filter.clear(),
                Typed::Submit => typing = false,
                Typed::Cancel => {
                    filter.clear();
                    typing = false;
                }
                Typed::Other => {}
            }
            continue;
        }
        match pager::key(tty)? {
            Key::Down => cursor = (cursor + 1).min(last),
            Key::Up => cursor = cursor.saturating_sub(1),
            Key::Next | Key::PageDown => cursor = (cursor + view).min(last),
            Key::Back | Key::PageUp => cursor = cursor.saturating_sub(view),
            Key::First => cursor = 0,
            Key::Last => cursor = last,
            Key::Search => typing = true,
            Key::Enter if !kept.is_empty() => return Ok(Some(kept[cursor])),
            Key::Escape if !filter.is_empty() => filter.clear(),
            Key::Quit | Key::Escape => return Ok(None),
            _ => {}
        }
    }
}

pub fn choose(items: &[Item], styled: bool) -> io::Result<Option<usize>> {
    let mut raw = Raw::enter()?;
    raw.tty.write_all(ENTER.as_bytes())?;
    let chosen = turn(items, &mut raw.tty, styled);
    let _ = raw.tty.write_all(format!("{RESET}{LEAVE}").as_bytes());
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(path: &str, text: &str) -> Block {
        Block {
            path: path.as_bytes().to_vec(),
            quality: 0.0,
            depth: 1,
            modified: None,
            line: None,
            text: text.as_bytes().to_vec(),
        }
    }

    fn sample() -> Vec<Item> {
        let blocks = [
            block("src/api", "src/api/\n"),
            block(
                "src/auth.ts",
                "src/auth.ts\nsrc/auth.ts:3:  // TODO: rotate\nsrc/auth.ts:12:\tlogin(a:b)\n",
            ),
            block("a:b.md", "a:b.md:7:see a:b\n"),
        ];
        items(blocks.iter(), false)
    }

    #[test]
    fn every_printed_row_becomes_a_choice() {
        let items = sample();
        let plain: Vec<&str> = items.iter().map(|item| item.plain.as_str()).collect();
        assert_eq!(
            plain,
            [
                "src/api/",
                "src/auth.ts",
                "src/auth.ts:3  // TODO: rotate",
                "src/auth.ts:12  login(a:b)",
                "a:b.md:7  see a:b"
            ]
        );
        let lines: Vec<Option<u64>> = items.iter().map(|item| item.line).collect();
        assert_eq!(lines, [None, None, Some(3), Some(12), Some(7)]);
        assert_eq!(items[0].path, b"src/api");
        assert_eq!(items[4].path, b"a:b.md");
    }

    #[test]
    fn the_filter_keeps_rows_with_every_word() {
        let items = sample();
        assert_eq!(narrowed(&items, ""), [0, 1, 2, 3, 4]);
        assert_eq!(narrowed(&items, "auth"), [1, 2, 3]);
        assert_eq!(narrowed(&items, "auth todo"), [2]);
        assert_eq!(narrowed(&items, "auth Todo"), Vec::<usize>::new());
        assert_eq!(narrowed(&items, "zz"), Vec::<usize>::new());
    }

    #[test]
    fn the_cursor_row_is_marked_and_the_rest_is_styled() {
        let blocks = [block("a.ts", "a.ts:2:x\n"), block("b.ts", "b.ts\n")];
        let items = items(blocks.iter(), true);
        let shown = rows(&items, &[0, 1], 0, 5, 1);
        assert_eq!(
            shown[0],
            format!("{MARGIN}{BOLD}a.ts{RESET}{NUMBER}:2{RESET}  x")
        );
        assert_eq!(shown[1], format!("{HIT}{POINTER}b.ts{HIT_END}"));
        assert_eq!(rows(&items, &[0, 1], 1, 5, 1).len(), 1);
    }

    #[test]
    fn the_bottom_line_shows_the_place_and_the_filter() {
        let look = Look {
            columns: 120,
            styled: false,
        };
        assert_eq!(
            footer("", false, 2, 9, 9, look),
            "  3/9   Enter choose   j k move   / filter   q cancel"
        );
        assert_eq!(
            footer("au", false, 0, 3, 9, look),
            "  1/3   /au   Enter choose   j k move   / filter   Esc clear   q cancel"
        );
        assert_eq!(
            footer("au", true, 0, 3, 9, look),
            format!("  /au{HIT} {HIT_END}   3 of 9   Enter keep   Esc clear")
        );
    }
}
