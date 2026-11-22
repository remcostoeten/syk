use crate::Fixture;

#[test]
fn bare_pattern_finds_folders_files_and_lines() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["needle"]),
        [
            "needle/",
            "src/api/auth.ts:2:const needle = 1",
            "src/api/route.ts:1:needle in route",
            "src/web/needle.md",
        ]
    );
}

#[test]
fn name_dir_and_content_flags_pick_one_target() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-n", "needle"]),
        ["needle/", "src/web/needle.md"]
    );
    assert_eq!(fixture.lines(&["-d", "needle"]), ["needle/"]);
    assert_eq!(
        fixture.lines(&["-c", "needle"]),
        [
            "src/api/auth.ts:2:const needle = 1",
            "src/api/route.ts:1:needle in route",
        ]
    );
    assert_eq!(
        fixture.lines(&["-n", "needle", "-t", "file"]),
        ["src/web/needle.md"]
    );
}

#[test]
fn separate_patterns_narrow_each_other() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-n", "route", "-c", "needle"]),
        ["src/api/route.ts:1:needle in route"]
    );
    assert_eq!(
        fixture.lines(&["-n", "route", "other"]),
        ["src/api/route.ts:2:other"]
    );
    assert_eq!(
        fixture.lines(&["-n", "keep", "needle"]),
        ["needle/keep.txt"]
    );
    assert_eq!(
        fixture.lines(&["-d", "api", "-c", "needle"]),
        [
            "src/api/auth.ts:2:const needle = 1",
            "src/api/route.ts:1:needle in route",
        ]
    );
    assert_eq!(
        fixture.lines(&["-d", "web", "-n", "route"]),
        ["src/web/route.tsx"]
    );
}

#[test]
fn listing_without_a_pattern() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-d"]),
        ["needle/", "src/", "src/api/", "src/web/"]
    );
    assert_eq!(
        fixture.lines(&[".ts"]),
        ["src/api/auth.ts", "src/api/route.ts"]
    );
    assert_eq!(fixture.lines(&["+1kb"]), ["notes.log"]);
    assert_eq!(
        fixture.lines(&["-n", "-D", "1"]),
        ["big.bin", "needle/", "notes.log", "src/"]
    );
}

#[test]
fn file_names_from_an_expanded_wildcard_are_searched_as_given() {
    let fixture = Fixture::new();
    fixture.file("one.cfg", "needle one\n");
    fixture.file("two.cfg", "nothing\n");
    fixture.file("src/three.cfg", "needle three\n");
    assert_eq!(
        fixture.lines(&["needle", "one.cfg", "two.cfg"]),
        ["one.cfg:1:needle one"]
    );
    assert_eq!(
        fixture.lines(&["one.cfg", "two.cfg"]),
        ["one.cfg", "two.cfg"]
    );
    assert_eq!(
        fixture.lines(&["needle", ".cfg"]),
        ["one.cfg:1:needle one", "src/three.cfg:1:needle three"]
    );
}

#[test]
fn exit_status_reports_the_outcome() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["needle"]).1, 0);
    assert_eq!(fixture.run(&["zzzmissing"]).1, 1);
    assert_eq!(fixture.run(&["--nope"]).1, 2);
    assert_eq!(fixture.run(&[]).1, 2);
}
