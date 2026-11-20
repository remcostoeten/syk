use std::ffi::OsString;
use std::io;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::process::CommandExt;
use std::process::Command;

const BATCH_BYTES: usize = 96 * 1024;
const BATCH_PATHS: usize = 4096;

fn argument(path: &[u8]) -> OsString {
    let mut safe = Vec::with_capacity(path.len() + 2);
    if path.starts_with(b"-") {
        safe.extend_from_slice(b"./");
    }
    safe.extend_from_slice(path);
    OsString::from_vec(safe)
}

fn shell(script: &str, paths: &[OsString]) -> bool {
    let status = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .args(paths)
        .status();
    match status {
        Ok(status) => status.success(),
        Err(error) => {
            eprintln!("syk: could not start sh: {error}");
            false
        }
    }
}

fn batches(paths: &[OsString]) -> Vec<&[OsString]> {
    let mut batches = Vec::new();
    let mut start = 0;
    while start < paths.len() {
        let mut end = start + 1;
        let mut bytes = paths[start].len();
        while end < paths.len()
            && end - start < BATCH_PATHS
            && bytes + paths[end].len() < BATCH_BYTES
        {
            bytes += paths[end].len() + 1;
            end += 1;
        }
        batches.push(&paths[start..end]);
        start = end;
    }
    batches
}

pub fn execute(command: &str, paths: &[&[u8]]) -> bool {
    let paths: Vec<OsString> = paths.iter().map(|path| argument(path)).collect();
    let mut fine = true;
    if command.contains("{}") {
        let script = command.replace("{}", "\"$1\"");
        for path in &paths {
            fine &= shell(&script, std::slice::from_ref(path));
        }
    } else {
        let script = format!("{command} \"$@\"");
        for batch in batches(&paths) {
            fine &= shell(&script, batch);
        }
    }
    fine
}

fn located(program: &str, path: OsString, line: Option<u64>) -> Vec<OsString> {
    let name = program.rsplit('/').next().unwrap_or(program);
    let Some(line) = line else {
        return vec![path];
    };
    let at = |path: OsString| {
        let mut joined = path;
        joined.push(format!(":{line}"));
        joined
    };
    match name {
        "code" | "code-insiders" | "codium" | "vscodium" | "cursor" | "windsurf" => {
            vec!["--goto".into(), at(path)]
        }
        "zed" | "zeditor" | "subl" | "hx" | "helix" => vec![at(path)],
        "kate" | "kwrite" => vec!["--line".into(), line.to_string().into(), path],
        "vi" | "vim" | "nvim" | "gvim" | "mvim" | "lvim" | "view" | "nano" | "pico" | "emacs"
        | "emacsclient" | "micro" | "kak" | "joe" | "ne" | "mg" | "gedit" => {
            vec![format!("+{line}").into(), path]
        }
        _ => vec![path],
    }
}

pub fn open(path: &[u8], line: Option<u64>) -> (String, io::Error) {
    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| !value.trim().is_empty());
    let path = argument(path);
    let mut words = editor.iter().flat_map(|editor| editor.split_whitespace());
    let (program, arguments) = match words.next() {
        Some(program) => {
            let mut arguments: Vec<OsString> = words.map(OsString::from).collect();
            arguments.extend(located(program, path, line));
            (program.to_string(), arguments)
        }
        None => {
            let opener = if cfg!(target_os = "macos") {
                "open"
            } else {
                "xdg-open"
            };
            (opener.to_string(), vec![path])
        }
    };
    let error = Command::new(&program).args(arguments).exec();
    (program, error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(program: &str, line: Option<u64>) -> Vec<String> {
        located(program, OsString::from("src/a.rs"), line)
            .into_iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn each_editor_gets_the_line_in_its_own_way() {
        assert_eq!(words("nvim", Some(12)), ["+12", "src/a.rs"]);
        assert_eq!(words("/usr/bin/code", Some(12)), ["--goto", "src/a.rs:12"]);
        assert_eq!(words("hx", Some(12)), ["src/a.rs:12"]);
        assert_eq!(words("kate", Some(12)), ["--line", "12", "src/a.rs"]);
        assert_eq!(words("unknown", Some(12)), ["src/a.rs"]);
        assert_eq!(words("nvim", None), ["src/a.rs"]);
    }

    #[test]
    fn a_path_that_looks_like_a_flag_is_made_harmless() {
        assert_eq!(argument(b"-rf"), OsString::from("./-rf"));
        assert_eq!(argument(b"src/-rf"), OsString::from("src/-rf"));
    }

    #[test]
    fn many_paths_are_split_over_several_runs() {
        let paths: Vec<OsString> = (0..BATCH_PATHS + 5)
            .map(|index| OsString::from(format!("file-{index}")))
            .collect();
        let sizes: Vec<usize> = batches(&paths).iter().map(|batch| batch.len()).collect();
        assert_eq!(sizes, [BATCH_PATHS, 5]);
        assert!(batches(&[]).is_empty());
    }
}
