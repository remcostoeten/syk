use crate::{Fixture, Terminal};

#[test]
fn every_flag_has_a_help_page() {
    let fixture = Fixture::new();
    let (overview, status) = fixture.run(&["-h"]);
    assert_eq!(status, 0);
    for flag in [
        "name", "dir", "content", "ext", "size", "modified", "type", "depth",
    ] {
        assert!(
            overview.contains(&format!("--{flag}")),
            "overview lacks --{flag}"
        );
        let (page, status) = fixture.run(&[&format!("--{flag}"), "-h"]);
        assert_eq!(status, 0);
        assert!(page.contains(&format!("--{flag}")), "no page for --{flag}");
        assert!(page.contains("EXAMPLES"), "no examples for --{flag}");
    }
    assert!(
        fixture
            .run(&["-n", "-h"])
            .0
            .starts_with("\n  -n, --name [PATTERN]\n")
    );
}

#[test]
fn long_help_is_paged_and_answers_to_its_keys() {
    let fixture = Fixture::new();
    let mut terminal = Terminal::open(&fixture, &["-h"], false);
    terminal.wait_for("1/");
    terminal.wait_for("n next: ARGUMENTS");
    terminal.press("n", "b back: top");
    terminal.press("/", "type what to find");
    terminal.press("pattern", "/pattern");
    terminal.wait_for(" lines");
    terminal.press("\r", "n next match");
    terminal.press("n", "3/13");
    terminal.press("\x1b", "/ search");
    terminal.press("s", "SECTIONS");
    terminal.press("d", "PATTERNS");
    terminal.press("?", "KEYS");
    terminal.press("x", "/ search");
    terminal.press("-", "the letter of a flag");
    terminal.press("q", "There is no flag -q.");
    terminal.press("-", "the letter of a flag");
    terminal.press("S", "Esc close");
    assert!(
        terminal
            .seen
            .contains("Only files of at least, or at most, this size.")
    );
    terminal.press("\x1b", "s sections");
    let (_, status) = terminal.finish("q");
    assert_eq!(status, 0);
}

#[test]
fn help_that_fits_or_is_switched_off_is_printed_in_one_piece() {
    let fixture = Fixture::new();
    for args in [&["-h", "-G"][..], &["-V", "-h"][..]] {
        let mut terminal = Terminal::open(&fixture, args, false);
        terminal.wait_for("MORE HELP");
        let seen = terminal.seen.clone();
        let (_, status) = terminal.finish("");
        assert_eq!(status, 0);
        assert!(!seen.contains("\x1b[?1049h"), "{args:?} was paged");
    }
    let mut terminal = Terminal::open(&fixture, &["-h", "-G", "-B"], false);
    terminal.wait_for("USAGE");
    assert!(terminal.seen.contains("  syk "));
    assert!(!terminal.seen.contains("|__/"));
}
