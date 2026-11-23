use std::fs;
use std::time::Duration;

use crate::Fixture;

#[cfg(target_os = "linux")]
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
            .errors(&["-y", "-H"])
            .contains("-y does not go with")
    );
}

#[cfg(target_os = "linux")]
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
