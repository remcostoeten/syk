use crate::Fixture;

#[test]
fn hidden_and_ignored_entries_are_opt_in() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-c", "needle", "-H", "-l"]),
        [".hidden/secret.txt", "src/api/auth.ts", "src/api/route.ts"]
    );
    assert_eq!(
        fixture.lines(&["-c", "needle", "-I", "-l"]),
        ["ignored/skip.txt", "src/api/auth.ts", "src/api/route.ts"]
    );
    assert_eq!(fixture.lines(&["-c", "needle", "-a", "-l"]).len(), 4);
}
