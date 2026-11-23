use std::fs;
use std::time::{Duration, Instant};

use crate::{Fixture, Terminal};

#[test]
fn an_index_finds_what_a_walk_finds_while_the_tree_changes() {
    let fixture = Fixture::new();
    for package in 0..6 {
        for file in 0..4 {
            fixture.file(
                &format!("pkg{package}/src/file{file}.rs"),
                "needle in a package\n",
            );
        }
    }
    fixture.file("pkg0/.gitignore", "*.log\n");
    fixture.file("pkg0/debug.log", "needle\n");
    let same = |args: &[&str]| -> String {
        let run = |extra: &[&str]| {
            let output = fixture
                .command(&[args, extra, &["-T", "-p"]].concat())
                .output()
                .expect("run syk");
            let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            lines.sort();
            (lines, String::from_utf8_lossy(&output.stderr).into_owned())
        };
        let (indexed, stats) = run(&[]);
        let (walked, _) = run(&["-Y"]);
        assert_eq!(indexed, walked, "{args:?} differs with an index");
        assert!(
            !walked.is_empty() || args.contains(&"zzz"),
            "{args:?} found nothing"
        );
        stats
    };
    let queries: [&[&str]; 9] = [
        &["-n"],
        &["-d", "pkg"],
        &["-n", "^file[0-2]"],
        &["-n", "rs$", "-o"],
        &["-c", "needle", "-l"],
        &["-n", "file", "-E", "pkg1"],
        &["-n", "-D", "2"],
        &["-d", "src", "-n", "file3"],
        &["needle"],
    ];

    assert!(!same(&["-n"]).contains("index"));
    std::thread::sleep(Duration::from_millis(2100));
    let (built, status) = fixture.run(&["-y"]);
    assert_eq!(status, 0);
    assert!(built.starts_with("Indexed "), "{built}");
    for query in queries {
        let stats = same(query);
        assert!(
            stats.contains("The names came from the index."),
            "{query:?}: {stats}"
        );
    }

    fixture.file("pkg1/src/added.rs", "needle added\n");
    fixture.file("pkg5/deep/er/file9.rs", "needle deep\n");
    fs::remove_file(fixture.root.join("pkg2/src/file0.rs")).expect("remove a file");
    fs::rename(fixture.root.join("pkg3"), fixture.root.join("pkg9")).expect("rename a folder");
    fs::remove_dir_all(fixture.root.join("pkg4")).expect("remove a folder");
    fs::remove_file(fixture.root.join("pkg0/src/file1.rs")).expect("remove a file");
    fixture.dir("pkg0/src/file1.rs");
    for query in queries {
        let stats = same(query);
        let sorted = query.contains(&"-o");
        assert_eq!(stats.contains("read again"), !sorted, "{query:?}: {stats}");
    }

    fixture.file("pkg0/.gitignore", "*.rs\n");
    assert!(!same(&["-n", "file"]).contains("index"));
    assert_eq!(fixture.run(&["-y"]).1, 0);
    same(&["-n", "file"]);
    assert!(
        fixture
            .errors(&["-y", "-L"])
            .contains("-y does not go with")
    );
}

#[test]
fn files_that_cannot_match_are_not_read_until_they_change() {
    let fixture = Fixture::new();
    fixture.file("docs/a.md", "plain words\n");
    fixture.file("docs/b.md", "other words\n");
    fixture.file("docs/Cargo.lock", "locked thing\n");
    let search = |extra: &[&str]| -> (Vec<String>, String) {
        let output = fixture
            .command(&[&["-c", "needle", "-T", "-p"], extra].concat())
            .output()
            .expect("run syk");
        let mut lines: Vec<String> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        (lines, String::from_utf8_lossy(&output.stderr).into_owned())
    };
    std::thread::sleep(Duration::from_millis(2100));
    let (built, status) = fixture.run(&["-y"]);
    assert_eq!(status, 0);
    assert!(built.contains("and the contents of 10 files"), "{built}");

    let (found, stats) = search(&[]);
    assert_eq!(found, search(&["-Y"]).0);
    assert!(
        stats.contains("The index showed that 8 files could not match"),
        "{stats}"
    );
    assert!(!stats.contains("of the group lock"), "{stats}");
    assert!(!search(&["-Y"]).1.contains("could not match"));

    let path = fixture.root.join("docs/a.md");
    let before = fs::metadata(&path)
        .and_then(|metadata| metadata.modified())
        .expect("a modification time");
    fs::write(&path, "needle word\n").expect("write the same length");
    fs::File::options()
        .write(true)
        .open(&path)
        .and_then(|file| file.set_modified(before))
        .expect("put the time back");
    fixture.file("big.bin", "needle as text\n");
    fixture.file("docs/Cargo.lock", "needle locked\n");
    let (found, stats) = search(&[]);
    for line in ["docs/a.md:1:needle word", "big.bin:1:needle as text"] {
        assert!(found.contains(&line.to_string()), "{line} in {found:?}");
    }
    assert_eq!(found, search(&["-Y"]).0);
    assert!(
        stats.contains("The index showed that 5 files could not match"),
        "{stats}"
    );
    assert!(
        stats.contains("1 more line matches in 1 file of the group lock"),
        "{stats}"
    );

    let (found, stats) = search(&["-u", "lock"]);
    assert!(found.contains(&"docs/Cargo.lock:1:needle locked".to_string()));
    assert_eq!(found, search(&["-u", "lock", "-Y"]).0);
    assert!(stats.contains("5 files could not match"), "{stats}");
}

#[test]
fn an_index_with_many_changed_files_is_built_again() {
    let fixture = Fixture::new();
    for file in 0..200 {
        fixture.file(&format!("many/file{file}.txt"), "plain words\n");
    }
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(fixture.run(&["-y"]).1, 0);
    let place = || {
        let listing = fs::read_dir(fixture.data().join("syk/index")).expect("an index folder");
        let found = listing
            .flatten()
            .find(|entry| entry.path().extension().is_some_and(|kind| kind == "idx"));
        found.expect("an index").path()
    };
    let index = || {
        let metadata = fs::metadata(place()).expect("its metadata");
        metadata.modified().expect("a modification time")
    };
    let aged = index() - Duration::from_secs(120);
    fs::File::options()
        .write(true)
        .open(place())
        .and_then(|file| file.set_modified(aged))
        .expect("make the index older");
    let built = index();
    for file in 0..100 {
        fixture.file(&format!("many/file{file}.txt"), "other words\n");
    }
    std::thread::sleep(Duration::from_millis(1100));
    let stats = fixture.errors(&["-c", "needle", "-T", "-p", "-l"]);
    assert!(stats.contains("could not match"), "{stats}");
    assert_eq!(index(), built);
    for file in 100..180 {
        fixture.file(&format!("many/file{file}.txt"), "other words\n");
    }
    let (found, status) = fixture.run(&["-c", "needle", "-p", "-l"]);
    assert_eq!((found.lines().count(), status), (2, 0));
    let started = std::time::Instant::now();
    while index() == built && started.elapsed() < Duration::from_secs(20) {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(index() > built, "the index was not built again");
}

#[test]
fn stored_indexes_can_be_listed_and_removed() {
    let fixture = Fixture::new();
    let stored = |fixture: &Fixture| {
        fs::read_dir(fixture.data().join("syk/index"))
            .map(|listing| {
                listing
                    .flatten()
                    .filter(|entry| entry.path().extension().is_some_and(|kind| kind == "idx"))
                    .count()
            })
            .unwrap_or(0)
    };
    assert!(fixture.run(&["-A"]).0.contains("No indexes in"));

    assert_eq!(fixture.run(&["-y", "src/"]).1, 0);
    assert_eq!(fixture.run(&["-y", "needle/"]).1, 0);
    let (listed, status) = fixture.run(&["-A"]);
    assert_eq!(status, 0);
    assert!(listed.contains("2 indexes"), "{listed}");
    assert!(fixture.run(&["-A", "./src"]).0.contains("1 index,"));
    assert_eq!(fixture.run(&["-A", "./notes.log"]).1, 1);

    assert_eq!(fixture.run(&["-A", "clear", "./src"]).1, 0);
    assert_eq!(stored(&fixture), 1);

    fs::remove_dir_all(fixture.root.join("needle")).expect("remove folder");
    assert!(fixture.run(&["-A"]).0.contains("folder is gone"));
    assert!(
        fixture
            .run(&["-A", "prune"])
            .0
            .starts_with("Removed 1 index")
    );
    assert_eq!(stored(&fixture), 0);

    assert_eq!(fixture.run(&["-y"]).1, 0);
    assert!(
        fixture
            .run(&["-A", "clear"])
            .0
            .starts_with("Removed 1 index")
    );
    assert_eq!(stored(&fixture), 0);
    assert!(
        fixture
            .errors(&["-A", "wipe"])
            .contains("list, clear or prune")
    );
}

#[test]
fn clearing_indexes_refuses_other_flags() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["-y"]).1, 0);
    for args in [&["-A", "clear", "-n", "x"][..], &["-A", "clear", "+1kb"]] {
        assert_eq!(fixture.run(args).1, 2, "{args:?}");
    }
    assert!(fixture.run(&["-A"]).0.contains("1 index,"));
}

#[test]
fn a_folder_inside_an_indexed_tree_uses_its_index() {
    let fixture = Fixture::new();
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(fixture.run(&["-y"]).1, 0);
    let run = |args: &[&str]| {
        fixture
            .command(&[args, &["-T", "-p"]].concat())
            .output()
            .expect("run syk")
    };
    let inside = run(&["-n", "route", "src/"]);
    assert!(String::from_utf8_lossy(&inside.stderr).contains("from the index"));
    let walked = run(&["-n", "route", "src/", "-Y"]);
    let sorted = |bytes: &[u8]| {
        let mut lines: Vec<String> = String::from_utf8_lossy(bytes)
            .lines()
            .map(str::to_string)
            .collect();
        lines.sort();
        lines
    };
    assert_eq!(sorted(&inside.stdout), sorted(&walked.stdout));
    assert_eq!(fixture.run(&["-A"]).0.matches("1 index,").count(), 1);

    let ignored = run(&["-n", "skip", "ignored/"]);
    assert!(!String::from_utf8_lossy(&ignored.stderr).contains("from the index"));
}

#[cfg(unix)]
#[test]
fn an_index_repeats_the_errors_below_a_folder_inside_it() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new();
    fixture.file("src/locked/inner.txt", "needle\n");
    let locked = fixture.root.join("src/locked");
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).expect("lock");
    let readable = fs::read_dir(&locked).is_ok();
    std::thread::sleep(Duration::from_millis(2100));
    let built = fixture.run(&["-y"]).1;
    let errors = |extra: &[&str]| {
        let args = [&["-n", "route", "src/", "-T", "-p"][..], extra].concat();
        let output = fixture.command(&args).output().expect("run syk");
        let text = String::from_utf8_lossy(&output.stderr).into_owned();
        (text, output.status.code())
    };
    let (indexed, status) = errors(&[]);
    let (walked, walked_status) = errors(&["-Y"]);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).expect("unlock");
    assert_eq!(built, 0);
    assert!(indexed.contains("from the index"), "{indexed}");
    assert_eq!(status, walked_status);
    if !readable {
        let denied = |text: &str| text.lines().next().map(str::to_string);
        assert_eq!(denied(&indexed), denied(&walked));
        assert!(indexed.starts_with("syk: src/locked: "), "{indexed}");
    }
}

#[test]
fn a_new_index_keeps_what_the_old_one_knew_of_unchanged_files() {
    let fixture = Fixture::new();
    std::thread::sleep(Duration::from_millis(2100));
    let first = fixture.run(&["-y"]).0;
    assert!(!first.contains("not read again"), "{first}");
    fixture.file("src/api/route.ts", "another needle in the route\n");
    std::thread::sleep(Duration::from_millis(2100));
    let second = fixture.run(&["-y"]).0;
    assert!(second.contains("had not changed"), "{second}");
    assert!(second.contains("not listed again"), "{second}");
    let found = |args: &[&str]| {
        let mut lines = fixture.lines(args);
        lines.sort();
        lines
    };
    for pattern in ["another needle", "needle", "login", "nothing here"] {
        let indexed = found(&["-c", pattern, "-l"]);
        assert_eq!(indexed, found(&["-c", pattern, "-l", "-Y"]), "{pattern}");
        assert!(!indexed.is_empty(), "{pattern}");
    }
}

#[test]
fn a_search_with_hidden_entries_has_an_index_of_its_own() {
    let fixture = Fixture::new();
    std::thread::sleep(Duration::from_millis(2100));
    assert_eq!(fixture.run(&["-y"]).1, 0);
    let stats = |args: &[&str]| {
        let output = fixture
            .command(&[args, &["-T", "-p"]].concat())
            .output()
            .expect("run syk");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        )
    };
    assert!(!stats(&["-H", "-n", "secret"]).1.contains("from the index"));

    assert_eq!(fixture.run(&["-y", "-H"]).1, 0);
    let (found, said) = stats(&["-H", "-n", "secret"]);
    assert!(said.contains("from the index"), "{said}");
    assert_eq!(found, ".hidden/secret.txt\n");
    assert!(stats(&["-n", "secret"]).0.is_empty());
    assert!(stats(&["-a", "-n", "skip"]).1.contains("Read 0 files"));
    assert!(!stats(&["-a", "-n", "skip"]).1.contains("from the index"));

    let listed = fixture.run(&["-A"]).0;
    assert!(listed.contains(" with -H\n"), "{listed}");
    assert!(listed.contains("2 indexes"), "{listed}");
}

#[test]
fn the_first_search_from_a_terminal_indexes_the_home_folder() {
    let mut fixture = Fixture::new();
    fixture.home = true;
    let listed = |fixture: &Fixture| fixture.run(&["-A"]).0;
    assert_eq!(fixture.run(&["-n", "route", "src/"]).1, 0);
    assert!(listed(&fixture).contains("No indexes in"));
    if listed(&fixture).contains("by itself") {
        return;
    }

    let mut terminal = Terminal::open(&fixture, &["-n", "route", "src/"], true);
    terminal.wait_for("building an index of");
    terminal.wait_for("-Y in the config file switches them off.");
    assert_eq!(terminal.finish("").1, 0);
    let started = Instant::now();
    while !listed(&fixture).contains("1 index,") {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "no index was built"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let home = fs::canonicalize(&fixture.root).expect("a real path");
    assert!(listed(&fixture).contains(&format!("{}\n", home.display())));

    assert_eq!(fixture.run(&["-A", "clear"]).1, 0);
    let again = Terminal::open(&fixture, &["-n", "route", "src/"], true);
    let (_, status) = again.finish("");
    assert_eq!(status, 0);
    assert!(fixture.data().join("syk/index/told").exists());
}
