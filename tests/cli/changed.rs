use std::process::Command;

use crate::Fixture;

fn git(fixture: &Fixture, args: &[&str]) {
    let done = Command::new("git")
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.invalid",
        ])
        .args(args)
        .current_dir(&fixture.root)
        .env("HOME", fixture.data())
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run git");
    assert!(done.status.success(), "{args:?}");
}

fn repository() -> Fixture {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(fixture.root.join(".git")).expect("remove the bare folder");
    git(&fixture, &["init", "-q"]);
    git(&fixture, &["add", "."]);
    git(&fixture, &["commit", "-q", "-m", "first"]);
    fixture
}

fn sorted(fixture: &Fixture, args: &[&str]) -> Vec<String> {
    let mut lines = fixture.lines(args);
    lines.sort();
    lines
}

#[test]
fn only_changed_and_new_files_are_searched() {
    let fixture = repository();
    assert_eq!(fixture.run(&["needle", "-Q"]).1, 1);
    fixture.file("src/api/route.ts", "needle changed\n");
    fixture.file("src/new file.ts", "needle new\n");
    fixture.file(".hidden/secret.txt", "needle secret\n");
    fixture.file("ignored/skip.txt", "needle still ignored\n");
    assert_eq!(
        sorted(&fixture, &["-c", "needle", "--uncommitted", "-l"]),
        [".hidden/secret.txt", "src/api/route.ts", "src/new file.ts"]
    );
    assert_eq!(
        sorted(&fixture, &["-n", "-Q"]),
        [".hidden/secret.txt", "src/api/route.ts", "src/new file.ts"]
    );
    assert_eq!(
        sorted(&fixture, &["needle", "-Q", "src/api/"]),
        ["src/api/route.ts:1:needle changed"]
    );
    assert_eq!(
        sorted(&fixture, &["-c", "needle", "-Q", "-l", "-e", "typescript"]),
        ["src/api/route.ts", "src/new file.ts"]
    );
}

#[test]
fn a_search_below_the_top_of_the_repository_works() {
    let fixture = repository();
    fixture.file("src/api/auth.ts", "needle edited\n");
    let output = fixture
        .command(&["-c", "needle", "-Q", "-l"])
        .current_dir(fixture.root.join("src"))
        .output()
        .expect("run syk");
    assert_eq!(String::from_utf8_lossy(&output.stdout), "api/auth.ts\n");
}

#[test]
fn it_needs_a_repository_and_a_folder() {
    let fixture = Fixture::new();
    std::fs::remove_dir_all(fixture.root.join(".git")).expect("remove the bare folder");
    let outside = fixture.data();
    std::fs::create_dir_all(&outside).expect("a folder");
    let output = fixture
        .command(&["needle", "-Q", "-Y"])
        .env(
            "GIT_CEILING_DIRECTORIES",
            fixture.root.parent().expect("a parent"),
        )
        .output()
        .expect("run syk");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("not in a git repository"));
    assert!(fixture.errors(&["-Q", "-y"]).contains("cannot go with"));
}

#[test]
fn a_language_name_selects_its_extensions() {
    let fixture = Fixture::new();
    fixture.file("a.mts", "needle\n");
    fixture.file("web.config", "needle\n");
    fixture.file("b.py", "needle\n");
    assert_eq!(
        sorted(&fixture, &["-c", "needle", "-l", "-e", "typescript"]),
        ["a.mts", "src/api/auth.ts", "src/api/route.ts"]
    );
    assert_eq!(
        sorted(&fixture, &["-c", "needle", "-l", "-e", "config"]),
        ["web.config"]
    );
    assert_eq!(
        sorted(&fixture, &["-c", "needle", "-l", "-e", "ts"]),
        ["src/api/auth.ts", "src/api/route.ts"]
    );
}
