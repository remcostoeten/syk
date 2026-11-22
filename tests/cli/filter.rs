use crate::Fixture;

#[test]
fn filters_apply_to_a_search() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["needle", ".ts", "src/"]),
        [
            "src/api/auth.ts:2:const needle = 1",
            "src/api/route.ts:1:needle in route",
        ]
    );
    assert_eq!(
        fixture.lines(&["needle", "-e", "md,tsx"]),
        ["src/web/needle.md"]
    );
    assert_eq!(fixture.lines(&["needle", "+1d"]), Vec::<String>::new());
    assert_eq!(
        fixture.lines(&["-n", "needle", "-1h"]),
        ["src/web/needle.md"]
    );
}

#[test]
fn excluded_names_are_left_out() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["needle", "-E", "api"]),
        ["needle/", "src/web/needle.md"]
    );
    assert_eq!(
        fixture.lines(&["needle", "-E", "auth.ts,needle"]),
        ["src/api/route.ts:1:needle in route", "src/web/needle.md"]
    );
    assert_eq!(
        fixture.lines(&["needle", "-E", "*.ts", "-E", ".md"]),
        ["needle/"]
    );
    assert_eq!(
        fixture.lines(&["needle", "--exclude", "src/api"]),
        ["needle/", "src/web/needle.md"]
    );
    assert_eq!(
        fixture.lines(&["-c", "needle", "-I", "-l", "-E", "ignored"]),
        ["src/api/auth.ts", "src/api/route.ts"]
    );
    assert_eq!(fixture.lines(&["-n", "needle", "-E", "need"]).len(), 2);
    assert_eq!(fixture.run(&["needle", "-E", "a{"]).1, 2);
}

#[test]
fn excludes_apply_to_remembered_paths() {
    let fixture = Fixture::new();
    fixture.run(&["-n", "secret", "-H", "-1"]);
    assert_eq!(
        fixture.run(&["-n", "secret", "-r"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-r", "-E", ".hidden"]),
        (String::new(), 1)
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-r", "-E", "*.txt"]),
        (String::new(), 1)
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-r", "-E", "other"]).0,
        ".hidden/secret.txt\n"
    );
}
