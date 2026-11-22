use std::io::Read;
use std::process::Stdio;

use crate::Fixture;

#[test]
fn an_interrupted_search_stops_with_status_130() {
    let fixture = Fixture::new();
    let mut child = fixture
        .command(&["-c", "e", "-a", "/usr"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start syk");
    let mut stdout = child.stdout.take().expect("a pipe");
    let mut first = [0u8; 1];
    stdout.read_exact(&mut first).expect("some output");
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).expect("the rest");
    let output = child.wait_with_output().expect("wait for syk");
    assert_eq!(output.status.code(), Some(130));
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        errors.contains("stopped before the search finished"),
        "{errors}"
    );
    assert!(rest.is_empty() || rest.ends_with(b"\n"));
}
