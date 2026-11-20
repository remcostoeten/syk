use std::io::IsTerminal;

const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const LOGO: &str = "\x1b[1;36m";

const MARGIN: &str = "  ";
const GAP: &str = "   ";
const WIDTH: usize = 80;

const ART: [&str; 5] = [
    r"          _    ",
    r" ___ _  _| |__ ",
    r"(_-<| || | / / ",
    r"/__/ \_, |_\_\ ",
    r"     |__/      ",
];
const NAME_ROW: usize = 2;
const ABOUT_ROW: usize = 3;

pub const TAGLINE: &str = "Search folder names, file names and contents in one pass.";
pub const EXPLAIN: &str = "How these arguments are read.";
pub const PICKS: &str = "The paths you chose with -1 or -O.";
pub const SETUP: &str = "Tab completion and the jump command for your shell.";

#[derive(Clone, Copy)]
pub struct Look {
    pub banner: bool,
    pub styled: bool,
}

impl Look {
    pub fn of(stream: &impl IsTerminal) -> Look {
        let terminal = stream.is_terminal();
        Look {
            banner: terminal,
            styled: terminal && std::env::var_os("NO_COLOR").is_none(),
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

fn fitted(about: &str) -> String {
    let room = WIDTH - MARGIN.len() - ART[ABOUT_ROW].len() - GAP.len();
    if about.chars().count() <= room {
        return about.to_string();
    }
    let mut short: String = about.chars().take(room - 3).collect();
    short.push_str("...");
    short
}

pub fn header(about: &str, look: Look) -> String {
    if !look.banner {
        return String::new();
    }
    let mut out = String::with_capacity(512);
    out.push('\n');
    for (row, art) in ART.iter().enumerate() {
        out.push_str(MARGIN);
        match row {
            NAME_ROW => {
                put(&mut out, LOGO, art, look.styled);
                out.push_str(GAP);
                put(&mut out, BOLD, "syk", look.styled);
                out.push(' ');
                put(&mut out, DIM, env!("CARGO_PKG_VERSION"), look.styled);
            }
            ABOUT_ROW => {
                put(&mut out, LOGO, art, look.styled);
                out.push_str(GAP);
                out.push_str(&fitted(about));
            }
            _ => put(&mut out, LOGO, art.trim_end(), look.styled),
        }
        out.push('\n');
    }
    out
}

pub fn above(about: &str, look: Look) -> String {
    let mut out = header(about, look);
    if look.banner {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHOWN: Look = Look {
        banner: true,
        styled: false,
    };

    #[test]
    fn nothing_is_added_outside_a_terminal() {
        let piped = Look {
            banner: false,
            styled: false,
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
            styled: true,
        };
        let text = header(TAGLINE, look);
        assert!(text.contains(&format!("{BOLD}syk{RESET}")));
        assert!(text.contains(&format!("{DIM}{}{RESET}", env!("CARGO_PKG_VERSION"))));
        assert!(text.contains(LOGO));
        assert!(above(TAGLINE, look).ends_with("\n\n"));
    }
}
