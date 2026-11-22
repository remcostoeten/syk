use crate::Fixture;

#[test]
fn output_flags_shape_the_result() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["needle", "-l"]),
        [
            "needle/",
            "src/api/auth.ts",
            "src/api/route.ts",
            "src/web/needle.md"
        ]
    );
    assert_eq!(fixture.lines(&["-c", "needle", "-m", "1"]).len(), 1);
    assert_eq!(
        fixture.lines(&["-c", "other", "-C", "1"]),
        [
            "src/api/route.ts-1-needle in route",
            "src/api/route.ts:2:other",
            "src/api/route.ts-3-last",
        ]
    );
    assert_eq!(
        fixture.lines(&["needle", "-P", "src/web", "src/api/route.ts"]),
        [
            "src/web/needle.md",
            "src/api/route.ts",
            "1:needle in route",
            ""
        ]
    );
}

#[test]
fn matching_lines_can_be_counted() {
    let fixture = Fixture::new();
    fixture.file("src/api/twice.ts", "needle\nother\nneedle again\n");
    assert_eq!(
        fixture.lines(&["needle", "-N"]),
        [
            "needle/",
            "src/api/auth.ts:1",
            "src/api/route.ts:1",
            "src/api/twice.ts:2",
            "src/web/needle.md",
        ]
    );
    let totals = fixture.errors(&["-c", "needle", "-T"]);
    assert!(
        totals.contains("syk: 4 lines in 3 files matched. Read "),
        "{totals}"
    );
    assert!(totals.contains(" entries in "), "{totals}");
    assert!(fixture.errors(&["-c", "needle"]).is_empty());
}

#[test]
fn results_come_as_json_or_separated_by_nul_bytes() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-c", "needle", "-J", "src/api/auth.ts"]),
        [
            r#"{"type":"match","path":"src/api/auth.ts","line":2,"text":"const needle = 1","spans":[[6,12]]}"#
        ]
    );
    assert_eq!(
        fixture.lines(&["-d", "needle", "-J"]),
        [r#"{"type":"path","path":"needle","kind":"dir"}"#]
    );
    assert_eq!(
        fixture.lines(&["-c", "needle", "-N", "-J", "src/api/auth.ts"]),
        [r#"{"type":"count","path":"src/api/auth.ts","count":1}"#]
    );
    fixture.file("src/quo\"te.ts", "a\ttab \\ needle\n");
    assert_eq!(
        fixture.lines(&["-c", "tab", "-J"]),
        [
            r#"{"type":"match","path":"src/quo\"te.ts","line":1,"text":"a\ttab \\ needle","spans":[[2,5]]}"#
        ]
    );
    assert_eq!(
        fixture.run(&["-c", "needle in", "-0", "-o"]).0,
        "src/api/route.ts\0"
    );
}
