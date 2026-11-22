use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "syk-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let fixture = Fixture { root };
        fixture.dir(".git");
        fixture.file(".gitignore", "ignored/\n");
        fixture.file(".hidden/secret.txt", "needle hidden\n");
        fixture.file("ignored/skip.txt", "needle ignored\n");
        fixture.file(
            "src/api/auth.ts",
            "export function login() {}\nconst needle = 1\n",
        );
        fixture.file("src/api/route.ts", "needle in route\nother\nlast\n");
        fixture.file("src/web/route.tsx", "nothing here\n");
        fixture.file("src/web/needle.md", "title\n");
        fixture.file("needle/keep.txt", "plain\n");
        fixture.file("big.bin", "needle\0binary\n");
        fixture.file("notes.log", &"x".repeat(5000));
        fixture
    }

    fn data(&self) -> PathBuf {
        self.root.with_extension("data")
    }

    fn dir(&self, path: &str) {
        fs::create_dir_all(self.root.join(path)).expect("create folder");
    }

    fn file(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        fs::create_dir_all(path.parent().unwrap_or(Path::new("."))).expect("create parent");
        fs::write(path, text).expect("write file");
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_syk"));
        command
            .args(args)
            .current_dir(&self.root)
            .env("NO_COLOR", "1")
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_CONFIG_HOME", self.data())
            .env("_ZO_DATA_DIR", self.data())
            .env_remove("SYK_CONFIG")
            .env_remove("VISUAL")
            .env_remove("EDITOR");
        command
    }

    fn run(&self, args: &[&str]) -> (String, i32) {
        let output = self.command(args).output().expect("run syk");
        (
            String::from_utf8_lossy(&output.stdout).into_owned(),
            output.status.code().unwrap_or(-1),
        )
    }

    fn errors(&self, args: &[&str]) -> String {
        let output = self.command(args).output().expect("run syk");
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    fn outside(&self, path: &str, text: &str) -> PathBuf {
        let path = self.data().join(path);
        fs::create_dir_all(path.parent().expect("a parent folder")).expect("create parent");
        fs::write(&path, text).expect("write file");
        path
    }

    fn lines(&self, args: &[&str]) -> Vec<String> {
        let mut args = args.to_vec();
        args.push("-o");
        self.run(&args).0.lines().map(str::to_string).collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
        let _ = fs::remove_dir_all(self.data());
    }
}

#[test]
fn large_binary_and_wide_files_are_read_like_small_ones() {
    let fixture = Fixture::new();
    let filler = "plain text line that fills the file\n".repeat(2500);
    let write = |path: &str, bytes: Vec<u8>| {
        fs::write(fixture.root.join(path), bytes).expect("write file");
    };
    fixture.dir("edge");
    write(
        "edge/large.txt",
        format!("{filler}sought at the end\n").into_bytes(),
    );
    write(
        "edge/late.bin",
        format!("{filler}sought before a zero\n\0tail\n").into_bytes(),
    );
    write("edge/early.bin", b"sought\0\n".to_vec());
    write(
        "edge/exact.txt",
        format!("{}\nsought after one buffer\n", "x".repeat(65535)).into_bytes(),
    );
    write("edge/noeol.txt", b"sought without newline".to_vec());
    write("edge/empty.txt", Vec::new());
    let mut wide = vec![0xFF, 0xFE];
    wide.extend(
        "first\nsought in utf16\n"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    write("edge/wide.txt", wide);
    assert_eq!(
        fixture.lines(&["-c", "sought", "-p"]),
        [
            "edge/exact.txt:2:sought after one buffer",
            "edge/large.txt:2501:sought at the end",
            "edge/noeol.txt:1:sought without newline",
            "edge/wide.txt:2:sought in utf16",
        ]
    );
    assert_eq!(
        fixture.lines(&["-c", "sought", "-N"]),
        [
            "edge/exact.txt:1",
            "edge/large.txt:1",
            "edge/noeol.txt:1",
            "edge/wide.txt:1",
        ]
    );
}

#[cfg(target_os = "linux")]
fn forget(path: &Path) {
    use std::os::fd::AsRawFd;

    let file = File::open(path).expect("open file");
    file.sync_all().expect("write file to disk");
    unsafe {
        libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED);
    }
}

#[cfg(not(target_os = "linux"))]
fn forget(_: &Path) {}

#[test]
fn files_are_found_whether_or_not_their_data_is_in_memory() {
    for count in [40, 400] {
        let fixture = Fixture::new();
        let mut wanted = Vec::new();
        for index in 0..count {
            let path = format!("many/{}/{index}.txt", index % 7);
            fixture.file(&path, &format!("filler\nsought {index}\n"));
            forget(&fixture.root.join(&path));
            wanted.push(format!("{path}:2:sought {index}"));
        }
        wanted.sort();
        let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
        for _ in 0..2 {
            let (printed, status) = fixture.run(&["-c", "sought", "-p"]);
            let mut lines: Vec<&str> = printed.lines().collect();
            lines.sort();
            assert_eq!((lines, status), (wanted.clone(), 0));
        }
    }
}

#[test]
fn heavy_files_in_a_small_tree_are_all_searched() {
    let fixture = Fixture::new();
    let text = "sought on every line\n".repeat(20_000);
    let mut wanted = Vec::new();
    for index in 0..8 {
        fixture.file(&format!("heavy/{index}.txt"), &text);
        wanted.push(format!("heavy/{index}.txt:20000"));
    }
    let wanted: Vec<&str> = wanted.iter().map(String::as_str).collect();
    let (printed, status) = fixture.run(&["-c", "sought", "-N"]);
    let mut lines: Vec<&str> = printed.lines().collect();
    lines.sort();
    assert_eq!((lines, status), (wanted, 0));
}

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
fn named_groups_are_skipped_until_they_are_asked_for() {
    let fixture = Fixture::new();
    fixture.file("node_modules/pkg/index.js", "needle in a package\n");
    fixture.file("vendor/lib.js", "needle in vendor\n");
    fixture.file("Cargo.lock", "needle locked\n");
    fixture.file(".cache/page.txt", "needle cached\n");
    let listed = |args: &[&str]| fixture.lines(&[&["-c", "needle", "-l"], args].concat());
    let sources = ["src/api/auth.ts", "src/api/route.ts"];
    assert_eq!(
        listed(&[]),
        ["Cargo.lock", sources[0], sources[1], "vendor/lib.js"]
    );
    assert_eq!(
        listed(&["-u", "deps"]),
        [
            "Cargo.lock",
            "node_modules/pkg/index.js",
            sources[0],
            sources[1],
            "vendor/lib.js"
        ]
    );
    assert_eq!(listed(&["-g", "lock", "-g", "deps+=vendor"]), sources);
    assert_eq!(listed(&["-g", "mine=src/api,*.lock"]), ["vendor/lib.js"]);
    assert_eq!(
        listed(&["-H"]),
        [
            ".hidden/secret.txt",
            "Cargo.lock",
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

    fixture.outside("syk/config", "-g lock\n");
    assert_eq!(listed(&[]), [sources[0], sources[1], "vendor/lib.js"]);
    assert_eq!(listed(&["-u", "lock"]).len(), 4);
}

#[test]
fn patterns_are_forgiving() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["-c", "login("]),
        ["src/api/auth.ts:1:export function login() {}"]
    );
    assert_eq!(fixture.lines(&["*.tsx", "-n"]), ["src/web/route.tsx"]);
    assert_eq!(fixture.lines(&["-n", "*oute.ts"]), ["src/api/route.ts"]);
    assert_eq!(fixture.lines(&["-c", "NEEDLE"]), Vec::<String>::new());
    assert_eq!(fixture.lines(&["-c", "NEEDLE", "-i", "-l"]).len(), 2);
    assert_eq!(fixture.lines(&["-c", "need", "-w"]), Vec::<String>::new());
}

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

fn ranking_fixture() -> Fixture {
    let fixture = Fixture::new();
    for name in ["config", "config.ts", "app_config.ts", "myconfig.ts"] {
        fixture.file(&format!("pick/{name}"), "needle\n");
    }
    fixture
}

#[test]
fn ranking_orders_names_by_how_well_they_match() {
    let fixture = ranking_fixture();
    let ranked = |args: &[&str]| -> Vec<String> {
        fixture.run(args).0.lines().map(str::to_string).collect()
    };
    assert_eq!(
        ranked(&["-n", "config", "-r"]),
        [
            "pick/config",
            "pick/config.ts",
            "pick/app_config.ts",
            "pick/myconfig.ts",
        ]
    );
    assert_eq!(
        ranked(&["-n", "config", "-r", "-m", "2"]),
        ["pick/config", "pick/config.ts"]
    );
    assert_eq!(ranked(&["-n", "config", "-1"]), ["pick/config"]);
    assert_eq!(ranked(&["-c", "needle", "-1", "pick/"]).len(), 1);
    assert_eq!(fixture.run(&["-n", "zzzmissing", "-1"]), (String::new(), 1));
}

#[test]
fn a_picked_path_ranks_higher_afterwards() {
    let fixture = ranking_fixture();
    let best = |pattern: &str| fixture.run(&["-n", pattern, "-1", "-t", "file"]).0;
    assert_eq!(
        fixture.run(&["-n", "config", "-r", "-m", "1"]).0,
        "pick/config\n"
    );
    assert_eq!(best("myconfig"), "pick/myconfig.ts\n");
    assert_eq!(best("config"), "pick/myconfig.ts\n");
    assert_eq!(
        fixture.run(&["-n", "config", "-r", "-m", "1"]).0,
        "pick/myconfig.ts\n"
    );
}

#[test]
fn a_picked_hidden_path_is_found_without_asking_for_hidden_files() {
    let fixture = Fixture::new();
    assert_eq!(fixture.run(&["-n", "secret", "-r"]), (String::new(), 1));
    assert_eq!(
        fixture.run(&["-n", "secret", "-H", "-1"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-r"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(
        fixture.run(&["-n", "secret", "-1"]).0,
        ".hidden/secret.txt\n"
    );
    assert_eq!(fixture.run(&["-n", "secret", "-R"]), (String::new(), 1));
    assert_eq!(fixture.run(&["-d", "secret", "-r"]), (String::new(), 1));
    assert_eq!(
        fixture.run(&["-n", "secret", "-r", "src/"]),
        (String::new(), 1)
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

#[test]
fn every_flag_has_a_help_page() {
    let fixture = Fixture::new();
    let (overview, status) = fixture.run(&["-h"]);
    assert_eq!(status, 0);
    for flag in [
        "name", "dir", "content", "ext", "size", "modified", "type", "depth",
    ] {
        assert!(
            overview.contains(&format!("--{flag}")),
            "overview lacks --{flag}"
        );
        let (page, status) = fixture.run(&[&format!("--{flag}"), "-h"]);
        assert_eq!(status, 0);
        assert!(page.contains(&format!("--{flag}")), "no page for --{flag}");
        assert!(page.contains("EXAMPLES"), "no examples for --{flag}");
    }
    assert!(
        fixture
            .run(&["-n", "-h"])
            .0
            .starts_with("\n  -n, --name [PATTERN]\n")
    );
}

#[test]
fn scores_and_picks_can_be_inspected() {
    let fixture = ranking_fixture();
    let (empty, status) = fixture.run(&["-k"]);
    assert_eq!(status, 0);
    assert!(empty.starts_with("Nothing is remembered yet."));

    let (why, _) = fixture.run(&["-n", "config", "-W", "-m", "2"]);
    let rows: Vec<&str> = why.lines().collect();
    assert!(rows[0].starts_with("score = picked (max 4) + name (max 3)"));
    assert!(
        rows[1]
            .trim_start()
            .starts_with("score  picked   name  visited  recent  shallow  path")
    );
    assert!(rows[2].ends_with("  3.00     0.00    1.00     0.17  pick/config"));
    assert!(rows[3].ends_with("  2.70     0.00    1.00     0.17  pick/config.ts"));
    assert_eq!(rows.len(), 4);

    let (dry, _) = fixture.run(&["-n", "myconfig", "-1", "-W"]);
    assert!(dry.ends_with("pick/myconfig.ts\n"));
    assert!(
        fixture
            .run(&["-k"])
            .0
            .starts_with("Nothing is remembered yet.")
    );

    fixture.run(&["-n", "myconfig", "-1"]);
    let (picks, _) = fixture.run(&["-k"]);
    let listed: Vec<&str> = picks.lines().collect();
    assert!(
        listed[0]
            .trim_start()
            .starts_with("score  picks  last picked  path")
    );
    assert!(listed[1].starts_with("  4.00    1.0  just now   "));
    assert!(listed[1].ends_with("/pick/myconfig.ts"));
    let (after, _) = fixture.run(&["-n", "config", "-W", "-m", "1"]);
    assert!(
        after
            .lines()
            .nth(2)
            .is_some_and(|row| row.contains("  4.00   0.60 "))
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

#[test]
fn several_words_have_to_match_together() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.lines(&["route", "in"]),
        ["src/api/route.ts:1:needle in route"]
    );
    assert_eq!(
        fixture.lines(&["const", "= 1"]),
        ["src/api/auth.ts:2:const needle = 1"]
    );
    assert_eq!(
        fixture.lines(&["tsx", "route", "-n"]),
        ["src/web/route.tsx"]
    );
    assert_eq!(fixture.run(&["other", "zzmissing"]).1, 1);
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

#[test]
fn a_command_runs_on_the_results() {
    let fixture = Fixture::new();
    fixture.file("-odd name.ts", "needle too\n");
    assert_eq!(
        fixture.run(&["-c", "needle", "-f", "printf '[%s]'"]).0,
        "[./-odd name.ts][src/api/auth.ts][src/api/route.ts]"
    );
    assert_eq!(
        fixture
            .run(&["-c", "needle", "-f", "echo {} done", "-m", "2"])
            .0,
        "./-odd name.ts done\nsrc/api/auth.ts done\n"
    );
    assert_eq!(
        fixture.run(&["-c", "needle", "-f", "false"]),
        (String::new(), 1)
    );
    assert_eq!(
        fixture.run(&["-c", "zzmissing", "-f", "echo ran"]),
        (String::new(), 1)
    );
}

#[test]
fn the_best_match_opens_in_the_editor() {
    let fixture = Fixture::new();
    let editor = fixture.outside("bin/vim", "#!/bin/sh\necho \"$@\"\n");
    let mut chmod = Command::new("chmod");
    assert!(
        chmod
            .arg("+x")
            .arg(&editor)
            .status()
            .expect("chmod")
            .success()
    );
    let open = |args: &[&str], editor: &str| {
        let output = fixture
            .command(args)
            .env("EDITOR", editor)
            .output()
            .expect("run syk");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let vim = editor.display().to_string();
    assert_eq!(open(&["-c", "other", "-O"], &vim), "+2 src/api/route.ts\n");
    assert_eq!(open(&["-d", "needle", "-O"], &vim), "needle\n");
    assert_eq!(
        open(&["-c", "other", "-O"], "echo --wait"),
        "--wait src/api/route.ts\n"
    );
    assert!(fixture.run(&["-k"]).0.contains("src/api/route.ts"));
}

#[test]
fn defaults_come_from_the_config_file() {
    let fixture = Fixture::new();
    fixture.outside("syk/config", "# defaults\n-E route.ts\n--stats\n");
    assert_eq!(
        fixture.lines(&["-c", "needle"]),
        ["src/api/auth.ts:2:const needle = 1"]
    );
    assert_eq!(fixture.lines(&["-c", "needle", "-U"]).len(), 2);
    assert!(
        fixture
            .errors(&["-c", "needle"])
            .contains("1 line in 1 file")
    );
    assert!(
        fixture
            .run(&["-c", "needle", "-x"])
            .0
            .contains("defaults  -E route.ts --stats")
    );
    assert_eq!(fixture.run(&["-h"]).1, 0);
    assert_eq!(fixture.run(&[]).1, 2);

    fixture.outside("syk/config", "needle\n");
    assert_eq!(fixture.run(&["-c", "needle"]).1, 2);
    assert!(fixture.errors(&["-c", "needle"]).contains("is not a flag"));
    assert_eq!(fixture.run(&["-c", "needle", "-U"]).1, 0);
}

#[test]
fn shell_setup_is_printed() {
    let fixture = Fixture::new();
    let (script, status) = fixture.run(&["--init", "fish"]);
    assert_eq!(status, 0);
    assert!(script.starts_with("function s "));
    assert!(script.contains("complete -c syk -s n -l name -d "));
    assert!(fixture.run(&["--init", "zsh:j"]).0.starts_with("j() {"));
    assert_eq!(fixture.run(&["--init", "tcsh"]).1, 2);
}

struct Home {
    fixture: Fixture,
}

impl Home {
    fn new() -> Home {
        Home {
            fixture: Fixture::new(),
        }
    }

    fn path(&self, path: &str) -> PathBuf {
        self.fixture.data().join(path)
    }

    fn file(&self, path: &str, text: &str) {
        self.fixture.outside(path, text);
    }

    fn read(&self, path: &str) -> Option<String> {
        fs::read_to_string(self.path(path)).ok()
    }

    fn has(shell: &str) -> bool {
        Command::new(shell)
            .arg("-c")
            .arg("exit 0")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }

    fn setup(&self, target: &str, answers: &str) -> (String, i32) {
        let binary = Path::new(env!("CARGO_BIN_EXE_syk"));
        let folder = binary.parent().expect("a folder").display();
        let mut child = self
            .fixture
            .command(&["--setup", target])
            .env("HOME", self.fixture.data())
            .env("PATH", format!("{folder}:/usr/local/bin:/usr/bin:/bin"))
            .env_remove("ZDOTDIR")
            .env_remove("SHELL")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("run syk");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(answers.as_bytes())
            .expect("answer");
        let output = child.wait_with_output().expect("wait for syk");
        let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
        text.push_str(&String::from_utf8_lossy(&output.stderr));
        (text, output.status.code().unwrap_or(-1))
    }
}

const FISH_PARTIAL: &str = "#!/usr/bin/env fish\n\nif is_interactive; and command -v zoxide >/dev/null 2>&1\n    zoxide init fish | source\nend\n";
const FISH_HELPER: &str = "function is_interactive\n    status is-interactive\nend\n";

#[test]
fn setup_follows_a_layout_with_one_file_per_tool() {
    if !Home::has("fish") {
        return;
    }
    let home = Home::new();
    home.file("dots/.git/HEAD", "");
    home.file("dots/configs/fish/config.fish", "source ~/dots/cfg\n");
    fs::create_dir_all(home.path("fish")).expect("create folder");
    std::os::unix::fs::symlink(
        home.path("dots/configs/fish/config.fish"),
        home.path("fish/config.fish"),
    )
    .expect("link");
    home.file(
        "dots/cfg",
        &format!("{FISH_HELPER}for file in ~/dots/tools/fish/*.fish\n    source $file\nend\n"),
    );
    home.file("dots/tools/fish/zoxide.fish", FISH_PARTIAL);

    let (shown, status) = home.setup("fish", "n\n");
    assert_eq!(status, 1);
    assert!(
        shown.contains("create  ~/dots/tools/fish/syk.fish"),
        "{shown}"
    );
    assert!(shown.contains("Nothing was written."));
    assert_eq!(home.read("dots/tools/fish/syk.fish"), None);

    let (done, status) = home.setup("fish", "y\n");
    assert_eq!(status, 0, "{done}");
    assert!(done.contains("Done. A new fish has s"), "{done}");
    assert_eq!(
        home.read("dots/tools/fish/syk.fish").as_deref(),
        Some(
            "#!/usr/bin/env fish\n\nif is_interactive; and command -v syk >/dev/null 2>&1\n    syk --init fish | source\nend\n"
        )
    );

    let (again, status) = home.setup("fish", "");
    assert_eq!(status, 0);
    assert!(again.contains("Already set up: ~/dots/tools/fish/syk.fish line 4."));
}

#[test]
fn setup_adds_a_file_to_a_list_that_names_each_file() {
    if !Home::has("fish") {
        return;
    }
    let home = Home::new();
    home.file(
        "fish/config.fish",
        &format!("{FISH_HELPER}source ~/parts/zoxide.fish\nset -g after 1\n"),
    );
    home.file("parts/zoxide.fish", FISH_PARTIAL);
    let (done, status) = home.setup("fish:j", "y\n");
    assert_eq!(status, 0, "{done}");
    assert_eq!(
        home.read("fish/config.fish"),
        Some(format!(
            "{FISH_HELPER}source ~/parts/zoxide.fish\nsource ~/parts/syk.fish\nset -g after 1\n"
        ))
    );
    assert!(
        home.read("parts/syk.fish")
            .is_some_and(|text| text.contains("syk --init fish:j | source"))
    );
}

#[test]
fn setup_undoes_a_guess_that_is_not_loaded() {
    if !Home::has("fish") {
        return;
    }
    let home = Home::new();
    home.file("fish/config.fish", "set -g nothing 1\n");
    home.file("fish/unused/zoxide.fish", FISH_PARTIAL);
    let (done, status) = home.setup("fish", "y\ny\n");
    assert_eq!(status, 0, "{done}");
    assert!(done.contains("did not load it, so the change was undone"));
    assert_eq!(home.read("fish/unused/syk.fish"), None);
    assert_eq!(
        home.read("fish/conf.d/syk.fish").as_deref(),
        Some("if status is-interactive; and command -q syk\n    syk --init fish | source\nend\n")
    );
}

#[test]
fn setup_refuses_a_name_that_is_taken() {
    if !Home::has("fish") {
        return;
    }
    let home = Home::new();
    home.file("fish/config.fish", "function s\n    echo mine\nend\n");
    let (shown, status) = home.setup("fish", "y\n");
    assert_eq!(status, 2);
    assert!(shown.contains("already has a command called s"), "{shown}");
    assert_eq!(home.read("fish/conf.d/syk.fish"), None);
}

#[test]
fn setup_adds_a_line_to_zsh_and_bash() {
    let zoxide = "export A=1\nif (( $+commands[zoxide] )); then\n  eval \"$(zoxide init zsh)\"\nfi\nalias l=ls\n";
    if Home::has("zsh") {
        let home = Home::new();
        home.file(".zshrc", zoxide);
        let (done, status) = home.setup("zsh", "y\n");
        assert_eq!(status, 0, "{done}");
        assert_eq!(
            home.read(".zshrc").as_deref(),
            Some(
                "export A=1\nif (( $+commands[zoxide] )); then\n  eval \"$(zoxide init zsh)\"\nfi\ncommand -v syk >/dev/null 2>&1 && eval \"$(syk --init zsh)\"\nalias l=ls\n"
            )
        );
    }
    if Home::has("bash") {
        let home = Home::new();
        home.file(".bashrc", "alias l=ls");
        let (done, status) = home.setup("bash", "y\n");
        assert_eq!(status, 0, "{done}");
        assert_eq!(
            home.read(".bashrc").as_deref(),
            Some("alias l=ls\n\ncommand -v syk >/dev/null 2>&1 && eval \"$(syk --init bash)\"\n")
        );
    }
}

#[test]
fn setup_prints_instructions_for_an_agent() {
    let home = Home::new();
    let (brief, status) = home.setup("agent", "");
    assert_eq!(status, 0);
    for expected in [
        "- fish: `syk --init fish | source`",
        "- zsh: `eval \"$(syk --init zsh)\"`",
        "`bash -i -c 'type s >/dev/null 2>&1'` exits with status 0.",
        "## Verify",
    ] {
        assert!(brief.contains(expected), "{expected} is missing");
    }
    assert_eq!(home.setup("tcsh", "").1, 2);
}

const PATIENCE: Duration = Duration::from_secs(10);
const PAUSE: Duration = Duration::from_millis(20);

struct Terminal {
    screen: File,
    child: Child,
    seen: String,
    read: usize,
}

impl Terminal {
    fn open(fixture: &Fixture, args: &[&str], piped: bool) -> Terminal {
        let size = libc::winsize {
            ws_row: 24,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut screen, mut inside) = (0, 0);
        let opened = unsafe {
            libc::openpty(
                &mut screen,
                &mut inside,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "no pseudo-terminal");
        let sized = unsafe { libc::ioctl(screen, libc::TIOCSWINSZ, &size) };
        assert_eq!(sized, 0, "no window size");
        let flags = unsafe { libc::fcntl(screen, libc::F_GETFL) };
        let free = unsafe { libc::fcntl(screen, libc::F_SETFL, flags | libc::O_NONBLOCK) };
        assert_eq!(free, 0, "the terminal cannot be read without waiting");
        let inside = unsafe { File::from_raw_fd(inside) };
        let mut command = fixture.command(args);
        command
            .env("TERM", "xterm-256color")
            .env_remove("SYK_PAGER")
            .stdin(inside.try_clone().expect("copy the terminal"))
            .stderr(inside.try_clone().expect("copy the terminal"));
        match piped {
            true => command.stdout(Stdio::piped()),
            false => command.stdout(inside.try_clone().expect("copy the terminal")),
        };
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().expect("start syk");
        Terminal {
            screen: unsafe { File::from_raw_fd(screen) },
            child,
            seen: String::new(),
            read: 0,
        }
    }

    fn wait_for(&mut self, text: &str) {
        let started = Instant::now();
        loop {
            if let Some(at) = self.seen[self.read..].find(text) {
                self.read += at + text.len();
                return;
            }
            assert!(
                started.elapsed() < PATIENCE,
                "{text:?} never showed up after {:?}",
                self.seen[self.read..]
                    .chars()
                    .rev()
                    .take(300)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect::<String>()
            );
            let mut chunk = [0u8; 8192];
            match self.screen.read(&mut chunk) {
                Ok(read) if read > 0 => {
                    self.seen.push_str(&String::from_utf8_lossy(&chunk[..read]));
                }
                _ => std::thread::sleep(PAUSE),
            }
        }
    }

    fn press(&mut self, keys: &str, then: &str) {
        self.screen.write_all(keys.as_bytes()).expect("send keys");
        self.wait_for(then);
    }

    fn finish(mut self, keys: &str) -> (String, i32) {
        self.screen.write_all(keys.as_bytes()).expect("send keys");
        let started = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("wait for syk") {
                break status;
            }
            if started.elapsed() > PATIENCE {
                let _ = self.child.kill();
                panic!("syk did not exit after {keys:?}");
            }
            let mut chunk = [0u8; 8192];
            if !matches!(self.screen.read(&mut chunk), Ok(read) if read > 0) {
                std::thread::sleep(PAUSE);
            }
        };
        let mut printed = String::new();
        if let Some(mut stdout) = self.child.stdout.take() {
            stdout
                .read_to_string(&mut printed)
                .expect("read the output");
        }
        (printed, status.code().unwrap_or(-1))
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn long_help_is_paged_and_answers_to_its_keys() {
    let fixture = Fixture::new();
    let mut terminal = Terminal::open(&fixture, &["-h"], false);
    terminal.wait_for("1/");
    terminal.wait_for("n next: ARGUMENTS");
    terminal.press("n", "b back: top");
    terminal.press("/", "type what to find");
    terminal.press("pattern", "/pattern");
    terminal.wait_for(" lines");
    terminal.press("\r", "n next match");
    terminal.press("n", "3/13");
    terminal.press("\x1b", "/ search");
    terminal.press("s", "SECTIONS");
    terminal.press("d", "PATTERNS");
    terminal.press("?", "KEYS");
    terminal.press("x", "/ search");
    terminal.press("-", "the letter of a flag");
    terminal.press("y", "There is no flag -y.");
    terminal.press("-", "the letter of a flag");
    terminal.press("S", "Esc close");
    assert!(
        terminal
            .seen
            .contains("Only files of at least, or at most, this size.")
    );
    terminal.press("\x1b", "s sections");
    let (_, status) = terminal.finish("q");
    assert_eq!(status, 0);
}

#[test]
fn help_that_fits_or_is_switched_off_is_printed_in_one_piece() {
    let fixture = Fixture::new();
    for args in [&["-h", "-G"][..], &["-V", "-h"][..]] {
        let mut terminal = Terminal::open(&fixture, args, false);
        terminal.wait_for("MORE HELP");
        let seen = terminal.seen.clone();
        let (_, status) = terminal.finish("");
        assert_eq!(status, 0);
        assert!(!seen.contains("\x1b[?1049h"), "{args:?} was paged");
    }
    let mut terminal = Terminal::open(&fixture, &["-h", "-G", "-B"], false);
    terminal.wait_for("USAGE");
    assert!(terminal.seen.contains("  syk "));
    assert!(!terminal.seen.contains("|__/"));
}

#[test]
fn a_result_is_chosen_from_the_list() {
    let fixture = Fixture::new();
    let mut terminal = Terminal::open(&fixture, &["-c", "needle", "-b"], true);
    terminal.wait_for("Enter choose");
    terminal.press("/", "Esc clear");
    terminal.press("api/route", "1 of ");
    terminal.press("\r", "1/1");
    let (printed, status) = terminal.finish("\r");
    assert_eq!(printed, "src/api/route.ts\n");
    assert_eq!(status, 0);
    assert!(fixture.run(&["-k"]).0.contains("src/api/route.ts"));

    let mut terminal = Terminal::open(&fixture, &["-n", "route", "-b"], true);
    terminal.wait_for("Enter choose");
    terminal.press("j", "2/2");
    let (printed, status) = terminal.finish("q");
    assert_eq!((printed.as_str(), status), ("", 1));

    let (printed, status) = fixture.run(&["-n", "zzzz", "-b", "-X"]);
    assert_eq!((printed.as_str(), status), ("", 1));
}

#[test]
fn an_interrupted_search_stops_with_status_130() {
    let fixture = Fixture::new();
    let mut child = fixture
        .command(&["-c", "e", "-a", "/usr"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start syk");
    let mut stdout = child.stdout.take().expect("a pipe");
    let mut first = [0u8; 1];
    stdout.read_exact(&mut first).expect("some output");
    assert_eq!(unsafe { libc::kill(child.id() as i32, libc::SIGINT) }, 0);
    let mut rest = Vec::new();
    stdout.read_to_end(&mut rest).expect("the rest");
    let output = child.wait_with_output().expect("wait for syk");
    assert_eq!(output.status.code(), Some(130));
    let errors = String::from_utf8_lossy(&output.stderr);
    assert!(
        errors.contains("stopped before the search finished"),
        "{errors}"
    );
    assert!(rest.is_empty() || rest.ends_with(b"\n"));
}
