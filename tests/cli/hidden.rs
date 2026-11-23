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

#[test]
fn the_global_gitignore_is_read_from_where_git_keeps_it() {
    let fixture = Fixture::new();
    let listed = |extra: &[&str]| {
        let output = fixture
            .command(&[&["-n", "txt"], extra].concat())
            .env("HOME", fixture.data())
            .output()
            .expect("run syk");
        let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        lines
    };
    assert_eq!(listed(&[]), ["needle/keep.txt"]);
    fixture.outside("git/ignore", "keep.txt\n");
    assert!(listed(&[]).is_empty() && listed(&["-o"]).is_empty());
    assert_eq!(listed(&["-I"]).len(), 2);
    fixture.outside("git/config", "[core]\n\tExcludesFile = ~/other\n");
    fixture.outside("other", "nothing\n");
    assert_eq!(listed(&[]), ["needle/keep.txt"]);
    assert_eq!(listed(&[]), listed(&["-o"]));
    fixture.outside("other", "needle\n");
    assert!(listed(&[]).is_empty() && listed(&["-o"]).is_empty());
}
