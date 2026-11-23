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

#[test]
fn a_search_below_a_folder_obeys_the_rules_above_it() {
    let fixture = Fixture::new();
    fixture.file(".gitignore", "ignored/\n/src/web/*.md\nauth.ts\n*.log\n");
    fixture.file("src/.ignore", "web/route.tsx\n!api/kept.log\n");
    fixture.file("src/api/kept.log", "needle\n");
    fixture.file("src/api/dropped.log", "needle\n");
    fixture.file("src/api/.env", "needle\n");
    fixture.file("src/web/deep/.gitignore", "!*.md\n");
    fixture.file("src/web/deep/page.md", "needle\n");
    let walked = |args: &[&str]| {
        let (output, _) = fixture.run(args);
        let mut lines: Vec<String> = output.lines().map(str::to_string).collect();
        lines.sort();
        lines
    };
    assert_eq!(
        walked(&["-n", "src/web/"]),
        ["src/web/deep/", "src/web/deep/page.md"]
    );
    assert_eq!(
        walked(&["-n", "./src/api/"]),
        ["src/api/kept.log", "src/api/route.ts"]
    );
    let roots = [
        "./",
        "src/",
        "./src/",
        "src/api/",
        "src/web/",
        "src/web/deep/",
        "needle/",
    ];
    let choices: [&[&str]; 4] = [&[], &["-H"], &["-I"], &["-D", "1"]];
    for root in roots {
        for flags in choices {
            let args = [&["-n", root], flags].concat();
            let sorted = [&args[..], &["-o"]].concat();
            assert_eq!(walked(&args), walked(&sorted), "{args:?}");
            assert_eq!(fixture.errors(&args), fixture.errors(&sorted), "{args:?}");
        }
    }
}
