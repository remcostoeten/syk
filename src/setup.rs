use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use ignore::WalkBuilder;

use crate::banner::{self, Look};
use crate::shell::{DEFAULT_NAME, valid};

const TOOLS: [&str; 12] = [
    "zoxide", "starship", "atuin", "direnv", "fnm", "mise", "fzf", "mcfly", "navi", "thefuck",
    "pyenv", "rbenv",
];
const LARGEST: u64 = 256 * 1024;
const MOST_ENTRIES: usize = 20_000;
const DEEPEST_SOURCE: usize = 3;
const PATIENCE: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shell {
    Fish,
    Zsh,
    Bash,
}

impl Shell {
    fn named(text: &str) -> Option<Shell> {
        let name = text.rsplit('/').next().unwrap_or(text);
        match name.to_ascii_lowercase().as_str() {
            "fish" => Some(Shell::Fish),
            "zsh" => Some(Shell::Zsh),
            "bash" | "sh" => Some(Shell::Bash),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Shell::Fish => "fish",
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
        }
    }

    fn line(self, function: &str) -> String {
        let target = if function == DEFAULT_NAME {
            self.name().to_string()
        } else {
            format!("{}:{function}", self.name())
        };
        match self {
            Shell::Fish => format!("syk --init {target} | source"),
            Shell::Zsh | Shell::Bash => format!("eval \"$(syk --init {target})\""),
        }
    }

    fn guarded(self, function: &str) -> String {
        let line = self.line(function);
        match self {
            Shell::Fish => format!("status is-interactive; and command -q syk; and {line}"),
            Shell::Zsh | Shell::Bash => format!("command -v syk >/dev/null 2>&1 && {line}"),
        }
    }

    fn file(self, function: &str) -> String {
        match self {
            Shell::Fish => format!(
                "if status is-interactive; and command -q syk\n    {}\nend\n",
                self.line(function)
            ),
            Shell::Zsh | Shell::Bash => format!("{}\n", self.guarded(function)),
        }
    }

    fn folder(self, home: &Path) -> PathBuf {
        let set = |name: &str| {
            std::env::var_os(name)
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
        };
        match self {
            Shell::Fish => set("XDG_CONFIG_HOME")
                .unwrap_or_else(|| home.join(".config"))
                .join("fish"),
            Shell::Zsh => set("ZDOTDIR").unwrap_or_else(|| home.to_path_buf()),
            Shell::Bash => home.to_path_buf(),
        }
    }

    fn entry(self, home: &Path) -> PathBuf {
        self.folder(home).join(match self {
            Shell::Fish => "config.fish",
            Shell::Zsh => ".zshrc",
            Shell::Bash => ".bashrc",
        })
    }

    fn probe(self, function: &str) -> String {
        match self {
            Shell::Fish => format!("functions -q {function}"),
            Shell::Zsh => format!("whence -w {function} >/dev/null"),
            Shell::Bash => format!("type {function} >/dev/null 2>&1"),
        }
    }

    fn reads(self, path: &Path) -> bool {
        let extension = path.extension().and_then(|extension| extension.to_str());
        match (self, extension) {
            (_, None) => true,
            (Shell::Fish, Some(extension)) => extension == "fish",
            (_, Some(extension)) => matches!(extension, "zsh" | "sh" | "bash"),
        }
    }

    fn tool(self, line: &str) -> Option<&'static str> {
        let text = line.trim();
        if text.starts_with('#') {
            return None;
        }
        let rest = match self {
            Shell::Fish => {
                let (command, piped) = text.split_once('|')?;
                piped
                    .trim_start()
                    .starts_with("source")
                    .then_some(command)?
            }
            Shell::Zsh | Shell::Bash => {
                let at = text.find("$(").or_else(|| text.find("<("))?;
                let loads = ["eval", "source", "."]
                    .iter()
                    .any(|word| text[..at].split_whitespace().any(|part| part == *word));
                loads.then_some(&text[at + 2..])?
            }
        };
        let rest = rest.trim_start();
        let rest = rest.strip_prefix("command ").unwrap_or(rest).trim_start();
        let word: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '-')
            .collect();
        TOOLS.into_iter().find(|tool| *tool == word)
    }
}

struct Source {
    path: PathBuf,
    lines: Vec<String>,
}

impl Source {
    fn read(path: &Path) -> Option<Source> {
        let size = fs::metadata(path).ok()?.len();
        if size > LARGEST {
            return None;
        }
        let text = fs::read_to_string(path).ok()?;
        Some(Source {
            path: path.to_path_buf(),
            lines: text.lines().map(str::to_string).collect(),
        })
    }

    fn text(&self) -> String {
        let mut text = self.lines.join("\n");
        text.push('\n');
        text
    }
}

fn expand(word: &str, home: &Path) -> Option<PathBuf> {
    let word = word.trim_matches(['"', '\'']);
    let below = ["~/", "$HOME/", "${HOME}/"]
        .iter()
        .find_map(|prefix| word.strip_prefix(prefix));
    if below.unwrap_or(word).contains(['$', '(', '*', '`']) {
        return None;
    }
    match below {
        Some(rest) => Some(home.join(rest)),
        None => word.starts_with('/').then(|| PathBuf::from(word)),
    }
}

fn sourced(source: &Source, home: &Path) -> Vec<PathBuf> {
    source
        .lines
        .iter()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let first = words.next()?;
            matches!(first, "source" | ".").then(|| words.next())?
        })
        .filter_map(|word| expand(word, home))
        .collect()
}

fn top(folder: &Path, home: &Path) -> PathBuf {
    folder
        .ancestors()
        .take(4)
        .take_while(|above| *above != home && above.starts_with(home))
        .find(|above| above.join(".git").exists())
        .unwrap_or(folder)
        .to_path_buf()
}

fn gather(shell: Shell, home: &Path) -> Vec<Source> {
    let mut files: BTreeSet<PathBuf> = BTreeSet::new();
    let mut folders: BTreeSet<PathBuf> = BTreeSet::new();
    let mut queue = vec![(shell.entry(home), 0)];
    let mut first = true;
    while let Some((path, depth)) = queue.pop() {
        let Ok(real) = fs::canonicalize(&path) else {
            continue;
        };
        if !real.is_file() || !files.insert(real.clone()) {
            continue;
        }
        if std::mem::take(&mut first)
            && let Some(folder) = real.parent().filter(|folder| *folder != home)
        {
            folders.insert(top(folder, home));
        }
        if depth < DEEPEST_SOURCE
            && let Some(source) = Source::read(&real)
        {
            queue.extend(
                sourced(&source, home)
                    .into_iter()
                    .map(|next| (next, depth + 1)),
            );
        }
    }
    if shell == Shell::Fish
        && let Ok(drop_ins) = fs::canonicalize(shell.folder(home).join("conf.d"))
    {
        folders.insert(drop_ins);
    }
    for folder in folders {
        let walk = WalkBuilder::new(&folder)
            .hidden(false)
            .max_depth(Some(6))
            .filter_entry(|entry| {
                !matches!(entry.file_name().to_str(), Some(".git" | "node_modules"))
            })
            .build();
        for entry in walk.take(MOST_ENTRIES).filter_map(Result::ok) {
            let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
            if is_file
                && shell.reads(entry.path())
                && let Ok(real) = fs::canonicalize(entry.path())
            {
                files.insert(real);
            }
        }
    }
    files.iter().filter_map(|path| Source::read(path)).collect()
}

fn installed(sources: &[Source]) -> Option<(&Path, usize)> {
    sources.iter().find_map(|source| {
        let at = source.lines.iter().position(|line| {
            let line = line.trim();
            !line.starts_with('#') && (line.contains("syk --init") || line.contains("syk -z "))
        })?;
        Some((source.path.as_path(), at + 1))
    })
}

struct Neighbour<'a> {
    source: &'a Source,
    at: usize,
    tool: &'static str,
    own_file: bool,
}

fn neighbour(shell: Shell, sources: &[Source]) -> Option<Neighbour<'_>> {
    sources
        .iter()
        .flat_map(|source| {
            let stem = source
                .path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default();
            source
                .lines
                .iter()
                .enumerate()
                .filter_map(move |(at, line)| {
                    let tool = shell.tool(line)?;
                    Some(Neighbour {
                        source,
                        at,
                        tool,
                        own_file: stem.contains(tool),
                    })
                })
        })
        .min_by_key(|found| {
            let rank = TOOLS.iter().position(|tool| *tool == found.tool);
            (!found.own_file, rank)
        })
}

fn swap_word(line: &str, from: &str, to: &str) -> String {
    let inside = |character: char| character.is_ascii_alphanumeric() || character == '_';
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find(from) {
        let before = rest[..at].chars().next_back();
        let after = rest[at + from.len()..].chars().next();
        out.push_str(&rest[..at]);
        if before.is_some_and(inside) || after.is_some_and(inside) {
            out.push_str(from);
        } else {
            out.push_str(to);
        }
        rest = &rest[at + from.len()..];
    }
    out.push_str(rest);
    out
}

fn indent(line: &str) -> &str {
    &line[..line.len() - line.trim_start().len()]
}

fn frame(text: &str, tool: &str) -> bool {
    let closes = matches!(text, "end" | "fi" | "}" | "then" | "else");
    let opens = ["if ", "if(", "[[ ", "[ ", "(( "]
        .iter()
        .any(|start| text.starts_with(start));
    let asks = ["command -v", "command -q", "type -q", "which ", "hash "]
        .iter()
        .any(|check| text.contains(check));
    closes || opens || (asks && text.contains(tool))
}

fn mirror(found: &Neighbour<'_>, line: &str) -> Option<String> {
    let mut out = String::new();
    for (at, original) in found.source.lines.iter().enumerate() {
        let text = original.trim();
        if at == found.at {
            out.push_str(indent(original));
            out.push_str(line);
        } else if text.is_empty() || text.starts_with("#!") {
            out.push_str(original);
        } else if text.starts_with('#') {
            continue;
        } else if frame(text, found.tool) {
            out.push_str(&swap_word(original, found.tool, "syk"));
        } else {
            return None;
        }
        out.push('\n');
    }
    Some(out)
}

struct Change {
    path: PathBuf,
    before: Option<String>,
    after: String,
    shown: String,
}

struct Plan {
    reason: String,
    changes: Vec<Change>,
}

fn added(source: &Source, after: usize, text: &str) -> Change {
    let mut lines = source.lines.clone();
    lines.insert(after + 1, text.to_string());
    let mut whole = lines.join("\n");
    whole.push('\n');
    Change {
        path: source.path.clone(),
        before: Some(source.text()),
        after: whole,
        shown: format!("after line {}:\n{text}", after + 1),
    }
}

fn alongside(
    shell: Shell,
    function: &str,
    sources: &[Source],
    found: &Neighbour<'_>,
) -> Option<Plan> {
    let theirs = found.source.path.file_name()?.to_str()?;
    let ours = theirs.replace(found.tool, "syk");
    let path = found.source.path.with_file_name(&ours);
    if path.exists() {
        return None;
    }
    let text = mirror(found, &shell.line(function)).unwrap_or_else(|| shell.file(function));
    let mut changes = vec![Change {
        path,
        before: None,
        after: text.clone(),
        shown: text,
    }];
    for source in sources {
        let listed = source
            .lines
            .iter()
            .position(|line| !line.trim().starts_with('#') && swap_word(line, theirs, "") != *line);
        if let Some(at) = listed.filter(|_| source.path != found.source.path) {
            let line = swap_word(&source.lines[at], theirs, &ours);
            changes.push(added(source, at, &line));
        }
    }
    Some(Plan {
        reason: format!(
            "{} has a file of its own there, so syk gets one next to it, written the same way.",
            found.tool
        ),
        changes,
    })
}

fn inline(shell: Shell, function: &str, found: &Neighbour<'_>) -> Plan {
    let lines = &found.source.lines;
    let depth = indent(&lines[found.at]).len();
    let closing = lines
        .iter()
        .enumerate()
        .skip(found.at + 1)
        .find(|(_, line)| {
            let text = line.trim();
            !text.is_empty() && indent(line).len() < depth
        });
    let (after, lead) = match closing {
        Some((at, line)) if depth > 0 && matches!(line.trim(), "end" | "fi" | "}") => {
            (at, indent(line))
        }
        _ => (found.at, indent(&lines[found.at])),
    };
    let text = format!("{lead}{}", shell.guarded(function));
    Plan {
        reason: format!(
            "{} is loaded from that file, so syk gets a line next to it.",
            found.tool
        ),
        changes: vec![added(found.source, after, &text)],
    }
}

fn standard(shell: Shell, function: &str, home: &Path) -> Plan {
    let (path, text) = match shell {
        Shell::Fish => (
            shell.folder(home).join("conf.d").join("syk.fish"),
            shell.file(function),
        ),
        Shell::Zsh | Shell::Bash => (shell.entry(home), shell.file(function)),
    };
    let before = fs::read_to_string(&path).ok();
    let mut after = before.clone().unwrap_or_default();
    if !after.is_empty() && !after.ends_with('\n') {
        after.push('\n');
    }
    if !after.is_empty() && shell != Shell::Fish {
        after.push('\n');
    }
    after.push_str(&text);
    Plan {
        reason: format!(
            "This is the standard place for {}: it is read at every start.",
            shell.name()
        ),
        changes: vec![Change {
            path,
            shown: if before.is_some() {
                format!("at the end:\n{text}")
            } else {
                text
            },
            before,
            after,
        }],
    }
}

fn plans(shell: Shell, function: &str, sources: &[Source], home: &Path) -> Vec<Plan> {
    let mut plans = Vec::new();
    if let Some(found) = neighbour(shell, sources) {
        let own = found
            .own_file
            .then(|| alongside(shell, function, sources, &found))
            .flatten();
        plans.push(own.unwrap_or_else(|| inline(shell, function, &found)));
    }
    let usual = standard(shell, function, home);
    let same = plans
        .iter()
        .any(|plan| plan.changes[0].path == usual.changes[0].path);
    if !same {
        plans.push(usual);
    }
    plans
}

fn short(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => path.display().to_string(),
    }
}

fn describe(plan: &Plan, home: &Path) -> String {
    let mut out = format!("{}\n", plan.reason);
    for change in &plan.changes {
        let verb = if change.before.is_some() {
            "change"
        } else {
            "create"
        };
        out.push_str(&format!("\n  {verb}  {}\n", short(&change.path, home)));
        for line in change.shown.lines() {
            out.push_str(&format!("      {line}\n"));
        }
    }
    out
}

fn apply(plan: &Plan) -> io::Result<()> {
    for change in &plan.changes {
        if let Some(folder) = change.path.parent() {
            fs::create_dir_all(folder)?;
        }
        fs::write(&change.path, &change.after)?;
    }
    Ok(())
}

fn undo(plan: &Plan) {
    for change in &plan.changes {
        let _ = match &change.before {
            Some(before) => fs::write(&change.path, before),
            None => fs::remove_file(&change.path),
        };
    }
}

fn loads(shell: Shell, function: &str) -> Option<bool> {
    let mut child = Command::new(shell.name())
        .args(["-i", "-c", &shell.probe(function)])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.success()),
            Ok(None) if started.elapsed() < PATIENCE => {
                std::thread::sleep(Duration::from_millis(20));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

fn agreed(question: &str) -> bool {
    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut answer = String::new();
    let read = io::stdin().lock().read_line(&mut answer).unwrap_or(0);
    if read == 0 || !io::stdin().is_terminal() {
        println!();
    }
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "j" | "ja"
    )
}

fn brief(shell: Option<Shell>, function: &str, home: &Path) -> String {
    let every = [Shell::Fish, Shell::Zsh, Shell::Bash];
    let shells: &[Shell] = match &shell {
        Some(shell) => std::slice::from_ref(shell),
        None => &every,
    };
    let mut out = String::from(
        "Set up the shell integration of syk, a command line search tool, in this user's shell configuration.\n\n## Goal\n\nEvery new interactive shell has to run one line at startup:\n\n",
    );
    for shell in shells {
        out.push_str(&format!("- {}: `{}`\n", shell.name(), shell.line(function)));
    }
    out.push_str(&format!(
        "\nThat line defines the function `{function}`, which changes to the best matching folder, and tab completion for syk.\n\n## Steps\n\n1. Check that it is not there already: search the shell configuration for `syk --init`. If it is, verify and stop.\n2. Read how the configuration is organised before you edit. Start at the file the shell reads first"
    ));
    for shell in shells {
        out.push_str(&format!(
            ", `{}` for {}",
            short(&shell.entry(home), home),
            shell.name()
        ));
    }
    out.push_str(
        ". Follow symbolic links and every file and folder it sources, including loaders that source a whole folder.\n3. Find how a comparable tool is loaded there, such as zoxide, starship, atuin, fnm, direnv or mise, and do the same for syk: the same folder, the same file naming, the same guards. A setup with one small file per tool gets one small file for syk.\n4. Without a comparable tool, use the standard place: a file in `conf.d` for fish, the end of `.zshrc` or `.bashrc` otherwise.\n5. Guard the line so that a machine without syk still starts cleanly, the way the neighbouring tools are guarded.\n6. Change as little as possible: one new file or one new line. Do not reorganise or reformat anything, and do not commit.\n\n## Verify\n\n",
    );
    for shell in shells {
        out.push_str(&format!(
            "- `{} -i -c '{}'` exits with status 0.\n",
            shell.name(),
            shell.probe(function)
        ));
    }
    out.push_str(
        "- `syk --setup` then reports that it is already set up.\n\nIf the check fails, the file you changed is not loaded at startup: undo the change and place the line where it is.\n\n## Report\n\nName the file you changed, show the lines you added, and give the result of the check.\n",
    );
    out
}

fn fail(message: &str) -> ExitCode {
    eprintln!("syk: {message}");
    ExitCode::from(2)
}

fn install(shell: Shell, function: &str, sources: &[Source], home: &Path) -> ExitCode {
    let name = shell.name();
    let entry = shell.entry(home);
    let real = fs::canonicalize(&entry).unwrap_or_else(|_| entry.clone());
    println!("{name} starts at {}.", short(&entry, home));
    if real != entry {
        println!("That is a link to {}.", short(&real, home));
    }
    for plan in plans(shell, function, sources, home) {
        println!("\n{}", describe(&plan, home));
        if !agreed("Write this?") {
            println!("Nothing was written.");
            return ExitCode::from(1);
        }
        if let Err(error) = apply(&plan) {
            undo(&plan);
            return fail(&format!("could not write the change: {error}"));
        }
        match loads(shell, function) {
            Some(true) => {
                println!(
                    "Done. A new {name} has {function} and completions for syk. Open a new shell and try: {function} <part of a folder name>"
                );
                return ExitCode::SUCCESS;
            }
            None => {
                println!(
                    "Written. {name} could not be started to check it, so open a new shell and try: {function} <part of a folder name>"
                );
                return ExitCode::SUCCESS;
            }
            Some(false) => {
                undo(&plan);
                println!("A new {name} did not load it, so the change was undone.");
            }
        }
    }
    println!(
        "\nNo place was found that your {name} reads at startup. Add this line by hand where it is read:\n\n  {}\n\nsyk --setup agent prints instructions that a coding agent can follow to do it.",
        shell.guarded(function)
    );
    ExitCode::from(1)
}

pub fn run(value: Option<&str>) -> ExitCode {
    if value.is_some_and(|value| value.eq_ignore_ascii_case("man")) {
        return match crate::manual::install() {
            Ok(message) => {
                println!("{message}");
                ExitCode::SUCCESS
            }
            Err(error) => fail(&error),
        };
    }
    let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_absolute())
    else {
        return fail("$HOME is not set, so the shell configuration cannot be found.");
    };
    let current = std::env::var("SHELL").ok();
    let given = value.or(current.as_deref()).unwrap_or("");
    let (which, function) = given.split_once(':').unwrap_or((given, DEFAULT_NAME));
    if which.is_empty() {
        return fail("$SHELL is not set, so name the shell: syk --setup fish");
    }
    if !valid(function) {
        return fail(&format!(
            "{function:?} cannot be the name of a shell function. Use letters, digits and underscores, for example: syk --setup fish:j"
        ));
    }
    if which.eq_ignore_ascii_case("agent") {
        let shell = current.as_deref().and_then(Shell::named);
        print!("{}", brief(shell, function, &home));
        return ExitCode::SUCCESS;
    }
    let Some(shell) = Shell::named(which) else {
        return fail(&format!(
            "there is no setup for {which:?}. Use fish, zsh or bash: syk --setup fish. For another shell, syk --setup agent prints instructions for a coding agent."
        ));
    };
    let name = shell.name();
    print!("{}", banner::above(banner::SETUP, Look::of(&io::stdout())));

    let sources = gather(shell, &home);
    if let Some((path, line)) = installed(&sources) {
        println!("Already set up: {} line {line}.", short(path, &home));
        return match loads(shell, function) {
            Some(false) => {
                println!(
                    "A new {name} does not have {function} though, so that file is not read at startup. Remove the line and run syk --setup again."
                );
                ExitCode::from(1)
            }
            _ => ExitCode::SUCCESS,
        };
    }
    if loads(shell, function) == Some(true) {
        return fail(&format!(
            "your {name} already has a command called {function}, and it is not from syk. Pick another name, for example: syk --setup {name}:j"
        ));
    }
    install(shell, function, &sources, &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(path: &str, text: &str) -> Source {
        Source {
            path: PathBuf::from(path),
            lines: text.lines().map(str::to_string).collect(),
        }
    }

    #[test]
    fn lines_that_load_another_tool_are_recognised() {
        assert_eq!(
            Shell::Fish.tool("    zoxide init fish | source"),
            Some("zoxide")
        );
        assert_eq!(
            Shell::Fish.tool("command starship init fish | source"),
            Some("starship")
        );
        assert_eq!(Shell::Fish.tool("# zoxide init fish | source"), None);
        assert_eq!(Shell::Fish.tool("echo zoxide | source"), None);
        assert_eq!(
            Shell::Zsh.tool("  eval \"$(zoxide init zsh)\""),
            Some("zoxide")
        );
        assert_eq!(Shell::Bash.tool("source <(fzf --bash)"), Some("fzf"));
        assert_eq!(Shell::Zsh.tool("export X=\"$(zoxide query x)\""), None);
        assert_eq!(Shell::Zsh.tool("eval \"$(unknown init zsh)\""), None);
    }

    #[test]
    fn a_file_per_tool_is_copied_in_the_same_style() {
        let sources = [source(
            "/dots/tools/zoxide.fish",
            "#!/usr/bin/env fish\n\n# jump around\nif dotfiles_shell_is_interactive; and command -v zoxide >/dev/null 2>&1\n    zoxide init fish | source\nend\n",
        )];
        let found = neighbour(Shell::Fish, &sources).expect("a neighbour");
        assert!(found.own_file);
        assert_eq!(
            mirror(&found, &Shell::Fish.line("s")).as_deref(),
            Some(
                "#!/usr/bin/env fish\n\nif dotfiles_shell_is_interactive; and command -v syk >/dev/null 2>&1\n    syk --init fish | source\nend\n"
            )
        );
    }

    #[test]
    fn a_file_with_more_in_it_is_not_copied() {
        let sources = [source(
            "/dots/tools/zoxide.fish",
            "set -gx _ZO_ECHO 1\nzoxide init fish | source\n",
        )];
        let found = neighbour(Shell::Fish, &sources).expect("a neighbour");
        assert_eq!(mirror(&found, &Shell::Fish.line("s")), None);
    }

    #[test]
    fn a_line_in_a_larger_file_goes_after_the_guard_around_it() {
        let sources = [source(
            "/home/me/.zshrc",
            "export A=1\nif (( $+commands[zoxide] )); then\n  eval \"$(zoxide init zsh)\"\nfi\nalias l=ls\n",
        )];
        let found = neighbour(Shell::Zsh, &sources).expect("a neighbour");
        assert!(!found.own_file);
        let plan = inline(Shell::Zsh, "j", &found);
        assert_eq!(
            plan.changes[0].after,
            "export A=1\nif (( $+commands[zoxide] )); then\n  eval \"$(zoxide init zsh)\"\nfi\ncommand -v syk >/dev/null 2>&1 && eval \"$(syk --init zsh:j)\"\nalias l=ls\n"
        );
    }

    #[test]
    fn only_whole_words_are_swapped() {
        assert_eq!(
            swap_word("command -v zoxide; my_zoxide zoxide.fish", "zoxide", "syk"),
            "command -v syk; my_zoxide syk.fish"
        );
    }

    #[test]
    fn sourced_paths_are_followed_when_they_are_literal() {
        let home = Path::new("/home/me");
        let file = source(
            "/home/me/.zshrc",
            "source ~/dots/init.zsh\n. \"$HOME/x.sh\"\nsource $ZSH/oh-my-zsh.sh\nsource /etc/profile\n",
        );
        assert_eq!(
            sourced(&file, home),
            [
                PathBuf::from("/home/me/dots/init.zsh"),
                PathBuf::from("/home/me/x.sh"),
                PathBuf::from("/etc/profile"),
            ]
        );
    }

    #[test]
    fn an_existing_setup_is_found() {
        let sources = [
            source("/a.fish", "# syk --init fish | source\n"),
            source("/b.fish", "\nsyk --init fish | source\n"),
        ];
        assert_eq!(installed(&sources), Some((Path::new("/b.fish"), 2)));
    }
}
