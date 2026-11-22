use crate::Fixture;

#[test]
fn named_groups_are_skipped_until_they_are_asked_for() {
    let fixture = Fixture::new();
    fixture.file("node_modules/pkg/index.js", "needle in a package\n");
    fixture.file("vendor/lib.js", "needle in vendor\n");
    fixture.file("app.min.js", "needle minified\n");
    fixture.file(".cache/page.txt", "needle cached\n");
    let listed = |args: &[&str]| fixture.lines(&[&["-c", "needle", "-l"], args].concat());
    let sources = ["src/api/auth.ts", "src/api/route.ts"];
    assert_eq!(
        listed(&[]),
        ["app.min.js", sources[0], sources[1], "vendor/lib.js"]
    );
    assert_eq!(
        listed(&["-u", "deps"]),
        [
            "app.min.js",
            "node_modules/pkg/index.js",
            sources[0],
            sources[1],
            "vendor/lib.js"
        ]
    );
    assert_eq!(listed(&["-g", "min", "-g", "deps+=vendor"]), sources);
    assert_eq!(listed(&["-g", "mine=src/api,*.min.js"]), ["vendor/lib.js"]);
    assert_eq!(
        listed(&["-H"]),
        [
            ".hidden/secret.txt",
            "app.min.js",
            sources[0],
            sources[1],
            "vendor/lib.js"
        ]
    );
    assert!(listed(&["-H", "-u", "all"]).contains(&".cache/page.txt".to_string()));
    let ignored = listed(&["-I"]);
    assert!(ignored.contains(&"node_modules/pkg/index.js".to_string()));
    assert!(ignored.contains(&"ignored/skip.txt".to_string()));
    assert_eq!(
        fixture.lines(&["-c", "needle", "-l", "node_modules"]),
        ["node_modules/pkg/index.js"]
    );
    assert!(
        fixture
            .errors(&["-c", "needle", "-g", "nope"])
            .contains("\"nope\" is not a group")
    );

    fixture.run(&["-n", "index", "-u", "deps", "-1"]);
    assert_eq!(fixture.run(&["-n", "index", "-r"]), (String::new(), 1));
    assert_eq!(
        fixture.run(&["-n", "index", "-r", "-u", "deps"]).0,
        "node_modules/pkg/index.js\n"
    );

    fixture.outside("syk/config", "-g min\n");
    assert_eq!(listed(&[]), [sources[0], sources[1], "vendor/lib.js"]);
    assert_eq!(listed(&["-u", "min"]).len(), 4);
}

#[test]
fn lock_files_are_found_by_name_but_their_contents_are_not_searched() {
    let fixture = Fixture::new();
    fixture.file("Cargo.lock", "needle locked\n");
    fixture.file("web/yarn.lock", "needle locked\n");
    fixture.file("fixtures/data.txt", "needle in a fixture\n");
    let sources = ["src/api/auth.ts", "src/api/route.ts"];
    let listed = |args: &[&str]| fixture.lines(&[&["-c", "needle", "-l"], args].concat());
    assert_eq!(listed(&[]), ["fixtures/data.txt", sources[0], sources[1]]);
    assert_eq!(
        listed(&["-u", "lock"]),
        [
            "Cargo.lock",
            "fixtures/data.txt",
            sources[0],
            sources[1],
            "web/yarn.lock"
        ]
    );
    assert_eq!(listed(&["-I"]).len(), 6);
    assert_eq!(
        fixture.lines(&["-c", "needle", "-l", "Cargo.lock"]),
        ["Cargo.lock"]
    );

    assert_eq!(
        fixture.lines(&["-n", "lock"]),
        ["Cargo.lock", "web/yarn.lock"]
    );
    assert_eq!(
        fixture.lines(&["-n", "lock", "-g", "lock"]),
        Vec::<String>::new()
    );
    assert_eq!(
        fixture.lines(&["locked"]),
        Vec::<String>::new(),
        "a bare pattern reads the contents of a lock file"
    );
    assert_eq!(fixture.lines(&["Cargo"]), ["Cargo.lock"]);
    assert_eq!(
        fixture.lines(&["-n", "lock", "-c", "needle"]),
        Vec::<String>::new()
    );
    assert_eq!(
        fixture.lines(&["-n", "lock", "-c", "needle", "-u", "lock"]),
        [
            "Cargo.lock:1:needle locked",
            "web/yarn.lock:1:needle locked"
        ]
    );

    let noted = |args: &[&str]| fixture.errors(&[&["-T"], args].concat());
    assert!(
        noted(&["-c", "needle"])
            .contains("2 more lines match in 2 files of the group lock, which is not shown. Add -u lock to see them.")
    );
    assert!(noted(&["-c", "locked"]).contains("2 lines match in 2 files of the group lock"));
    assert!(noted(&["-c", "needle", "-D", "1"]).contains(
        "1 line matches in 1 file of the group lock, which is not shown. Add -u lock to see it."
    ));
    for quiet in [
        &["-c", "needle", "-u", "lock"][..],
        &["-c", "needle", "-g", "lock"],
        &["-c", "plain"],
        &["-n", "lock"],
    ] {
        assert!(
            !noted(quiet).contains("not shown"),
            "{quiet:?} mentions lock files"
        );
    }
    assert!(fixture.errors(&["-c", "needle"]).is_empty());

    assert_eq!(
        listed(&["-g", "fixtures:content=fixtures"]),
        [sources[0], sources[1]]
    );
    assert_eq!(
        fixture.lines(&["-n", "data", "-g", "fixtures:content=fixtures"]),
        ["fixtures/data.txt"]
    );
    assert!(
        fixture
            .errors(&["-c", "needle", "-g", "mine:content=src/api"])
            .contains("src/api cannot be in the group mine")
    );
}
