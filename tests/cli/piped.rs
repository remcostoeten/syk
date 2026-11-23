use std::io::{Read, Write};
use std::process::{Output, Stdio};
use std::time::{Duration, Instant};

use crate::Fixture;

fn piped(fixture: &Fixture, args: &[&str], input: &[u8]) -> Output {
    let mut child = fixture
        .command(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start syk");
    let mut stdin = child.stdin.take().expect("a pipe");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output().expect("wait for syk");
    writer.join().expect("the writer");
    output
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn a_dash_searches_what_is_piped_in() {
    let fixture = Fixture::new();
    let input = b"first\nneedle one\nthird\nNEEDLE two\n";
    let found = piped(&fixture, &["needle", "-"], input);
    assert_eq!(found.status.code(), Some(0));
    assert_eq!(text(&found), "<stdin>:2:needle one\n<stdin>:4:NEEDLE two\n");
    let none = piped(&fixture, &["absent", "-"], input);
    assert_eq!(none.status.code(), Some(1));
    assert_eq!(text(&none), "");
}

#[test]
fn the_output_flags_work_on_a_pipe() {
    let fixture = Fixture::new();
    let input = b"a\nneedle\nb\nneedle\n";
    assert_eq!(
        text(&piped(&fixture, &["-c", "needle", "-", "-N"], input)),
        "<stdin>:2\n"
    );
    assert_eq!(
        text(&piped(&fixture, &["needle", "-", "-l"], input)),
        "<stdin>\n"
    );
    assert_eq!(
        text(&piped(&fixture, &["needle", "-", "-m", "1"], input)),
        "<stdin>:2:needle\n"
    );
    assert_eq!(
        text(&piped(&fixture, &["b", "-", "-C", "1", "-w"], input)),
        "<stdin>-2-needle\n<stdin>:3:b\n<stdin>-4-needle\n"
    );
    let json = text(&piped(&fixture, &["needle", "-", "-J", "-m", "1"], input));
    assert!(json.starts_with("{\"type\":\"match\",\"path\":\"<stdin>\",\"line\":2,"));
}

#[test]
fn a_dash_is_refused_where_a_pipe_has_no_meaning() {
    let fixture = Fixture::new();
    for args in [
        &["-"][..],
        &["-n", "needle", "-"],
        &["needle", "-", "src/"],
        &["needle", "-", ".ts"],
        &["needle", "-", "+1mb"],
        &["needle", "-", "-O"],
        &["needle", "-", "-y"],
    ] {
        let output = piped(&fixture, args, b"needle\n");
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains('-'));
    }
}

#[test]
fn a_dash_after_two_dashes_is_a_pattern() {
    let fixture = Fixture::new();
    fixture.file("dash.txt", "a - b\n");
    let (found, status) = fixture.run(&["-c", "--", "-"]);
    assert_eq!(status, 0);
    assert!(found.lines().any(|line| line == "dash.txt:1:a - b"));
}

#[test]
fn binary_input_stops_with_a_note() {
    let fixture = Fixture::new();
    let output = piped(&fixture, &["needle", "-"], b"\0needle\n");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("binary"));
}

#[test]
fn a_match_is_printed_before_the_input_ends() {
    let fixture = Fixture::new();
    let mut child = fixture
        .command(&["needle", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start syk");
    let mut stdin = child.stdin.take().expect("a pipe");
    let mut stdout = child.stdout.take().expect("a pipe");
    stdin.write_all(b"needle early\n").expect("write");
    stdin.flush().expect("flush");
    let started = Instant::now();
    let mut seen = [0; 7];
    stdout.read_exact(&mut seen).expect("the first result");
    assert_eq!(&seen, b"<stdin>");
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(stdin);
    assert_eq!(child.wait().expect("wait").code(), Some(0));
}
