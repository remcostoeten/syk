use std::process::Command;

use crate::{Fixture, Terminal};

#[test]
fn a_command_runs_on_the_results() {
    let fixture = Fixture::new();
    fixture.file("-odd name.ts", "needle too\n");
    assert_eq!(
        fixture.run(&["-c", "needle", "-f", "printf '[%s]'"]).0,
        "[./-odd name.ts][src/api/auth.ts][src/api/route.ts]"
    );
    assert_eq!(
        fixture
            .run(&["-c", "needle", "-f", "echo {} done", "-m", "2"])
            .0,
        "./-odd name.ts done\nsrc/api/auth.ts done\n"
    );
    assert_eq!(
        fixture.run(&["-c", "needle", "-f", "false"]),
        (String::new(), 1)
    );
    assert_eq!(
        fixture.run(&["-c", "zzmissing", "-f", "echo ran"]),
        (String::new(), 1)
    );
}

#[test]
fn the_best_match_opens_in_the_editor() {
    let fixture = Fixture::new();
    let editor = fixture.outside("bin/vim", "#!/bin/sh\necho \"$@\"\n");
    let mut chmod = Command::new("chmod");
    assert!(
        chmod
            .arg("+x")
            .arg(&editor)
            .status()
            .expect("chmod")
            .success()
    );
    let open = |args: &[&str], editor: &str| {
        let output = fixture
            .command(args)
            .env("EDITOR", editor)
            .output()
            .expect("run syk");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let vim = editor.display().to_string();
    assert_eq!(open(&["-c", "other", "-O"], &vim), "+2 src/api/route.ts\n");
    assert_eq!(open(&["-d", "needle", "-O"], &vim), "needle\n");
    assert_eq!(
        open(&["-c", "other", "-O"], "echo --wait"),
        "--wait src/api/route.ts\n"
    );
    assert!(fixture.run(&["-k"]).0.contains("src/api/route.ts"));
}

#[test]
fn a_result_is_chosen_from_the_list() {
    let fixture = Fixture::new();
    let mut terminal = Terminal::open(&fixture, &["-c", "needle", "-b"], true);
    terminal.wait_for("Enter choose");
    terminal.press("/", "Esc clear");
    terminal.press("api/route", "1 of ");
    terminal.press("\r", "1/1");
    let (printed, status) = terminal.finish("\r");
    assert_eq!(printed, "src/api/route.ts\n");
    assert_eq!(status, 0);
    assert!(fixture.run(&["-k"]).0.contains("src/api/route.ts"));

    let mut terminal = Terminal::open(&fixture, &["-n", "route", "-b"], true);
    terminal.wait_for("Enter choose");
    terminal.press("j", "2/2");
    let (printed, status) = terminal.finish("q");
    assert_eq!((printed.as_str(), status), ("", 1));

    let (printed, status) = fixture.run(&["-n", "zzzz", "-b", "-X"]);
    assert_eq!((printed.as_str(), status), ("", 1));
}
