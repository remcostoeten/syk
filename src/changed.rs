use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const WITHOUT_REPOSITORY_PROGRAMS: [&str; 4] =
    ["--no-optional-locks", "-c", "core.fsmonitor=false", "-C"];

fn git(folder: &Path, args: &[&str]) -> Result<Option<Vec<u8>>, String> {
    let output = Command::new("git")
        .args(WITHOUT_REPOSITORY_PROGRAMS)
        .arg(folder)
        .args(args)
        .output()
        .map_err(|error| format!("-Q, --uncommitted needs git: {error}"))?;
    Ok(output.status.success().then_some(output.stdout))
}

fn listed(status: &[u8]) -> impl Iterator<Item = &Path> {
    status
        .split(|byte| *byte == 0)
        .filter_map(|entry| entry.get(3..))
        .filter(|path| !path.is_empty())
        .map(|path| Path::new(OsStr::from_bytes(path)))
}

fn below(root: &Path, found: &mut HashSet<PathBuf>) -> Result<(), String> {
    let real = fs::canonicalize(root).map_err(|error| format!("{}: {error}", root.display()))?;
    let folder = match real.is_dir() {
        true => real.as_path(),
        false => real.parent().unwrap_or(&real),
    };
    let outside = || {
        format!(
            "{} is not in a git repository, so -Q, --uncommitted has nothing to compare.",
            root.display()
        )
    };
    let top = git(folder, &["rev-parse", "--show-toplevel"])?.ok_or_else(outside)?;
    let top = top.strip_suffix(b"\n").unwrap_or(&top);
    let top = fs::canonicalize(OsStr::from_bytes(top)).map_err(|_| outside())?;
    let status = git(
        &top,
        &[
            "status",
            "--porcelain",
            "-z",
            "--no-renames",
            "--untracked-files=all",
        ],
    )?
    .ok_or_else(|| format!("git could not read the status of {}.", top.display()))?;
    for path in listed(&status) {
        if let Ok(inside) = top.join(path).strip_prefix(&real) {
            found.insert(root.join(inside));
        }
    }
    Ok(())
}

pub fn load(roots: &[PathBuf]) -> Result<HashSet<PathBuf>, String> {
    let mut found = HashSet::new();
    for root in roots {
        below(root, &mut found)?;
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_status_is_read_with_spaces_and_without_empty_entries() {
        let status = b" M src/a b.rs\0?? new.txt\0D  gone.rs\0";
        let paths: Vec<&Path> = listed(status).collect();
        assert_eq!(
            paths,
            [
                Path::new("src/a b.rs"),
                Path::new("new.txt"),
                Path::new("gone.rs")
            ]
        );
    }
}
