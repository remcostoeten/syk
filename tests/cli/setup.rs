use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::Fixture;

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
