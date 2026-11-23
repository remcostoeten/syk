use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::build::prune;
use crate::index::{Index, now, store};
use crate::report::{counted, grouped};

const SECOND: i64 = 1_000_000_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    List,
    Clear,
    Prune,
}

impl Action {
    pub fn of(text: &str) -> Result<Action, String> {
        match text {
            "" | "list" => Ok(Action::List),
            "clear" => Ok(Action::Clear),
            "prune" => Ok(Action::Prune),
            other => Err(format!(
                "{other:?} is not something -A does. Use list, clear or prune, and write a folder as a path, such as {other}/."
            )),
        }
    }
}

struct Held {
    file: PathBuf,
    root: Option<PathBuf>,
    view: &'static str,
    entries: u64,
    built: i64,
    size: u64,
}

impl Held {
    fn read(file: PathBuf) -> Held {
        let size = [file.clone(), file.with_extension("sig")]
            .iter()
            .filter_map(|path| fs::metadata(path).ok())
            .map(|found| found.len())
            .sum();
        let index = Index::load(&file);
        Held {
            root: index.as_ref().map(|index| index.root().to_path_buf()),
            view: index.as_ref().map_or("", Index::view),
            entries: index.as_ref().map_or(0, |index| u64::from(index.entries())),
            built: index.as_ref().map_or(0, Index::built),
            size,
            file,
        }
    }

    fn lapsed(&self) -> bool {
        !self.root.as_deref().is_some_and(Path::is_dir)
    }

    fn label(&self) -> String {
        let view = match self.view {
            "" => String::new(),
            view => format!(" with {view}"),
        };
        match &self.root {
            Some(root) if root.is_dir() => format!("{}{view}", root.display()),
            Some(root) => format!("{}{view} (folder is gone)", root.display()),
            None => format!("{} (unreadable)", self.file.display()),
        }
    }

    fn remove(&self) -> bool {
        let removed = fs::remove_file(&self.file).is_ok();
        for kind in ["sig", "no"] {
            let _ = fs::remove_file(self.file.with_extension(kind));
        }
        removed
    }
}

fn held(store: &Path) -> Vec<Held> {
    let Ok(listing) = fs::read_dir(store) else {
        return Vec::new();
    };
    let mut found: Vec<Held> = listing
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|kind| kind == "idx"))
        .map(Held::read)
        .collect();
    found.sort_by_key(|item| std::cmp::Reverse(item.size));
    found
}

fn real(path: &Path) -> PathBuf {
    fs::canonicalize(path)
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn bytes(size: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if size < 1000 {
        return format!("{size} B");
    }
    let mut value = size as f64 / 1000.0;
    let mut unit = 0;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    match value < 10.0 {
        true => format!("{value:.1} {}", UNITS[unit]),
        false => format!("{value:.0} {}", UNITS[unit]),
    }
}

fn age(built: i64) -> String {
    let seconds = (now() - built).max(0) / SECOND;
    match seconds {
        0..60 => "just now".to_string(),
        60..3600 => format!("{} min ago", seconds / 60),
        3600..86400 => format!("{} h ago", seconds / 3600),
        _ => format!("{} d ago", seconds / 86400),
    }
}

fn list(store: &Path, items: &[Held]) -> ExitCode {
    if items.is_empty() {
        println!("No indexes in {}.", store.display());
        return ExitCode::SUCCESS;
    }
    println!("Indexes in {}\n", store.display());
    println!("{:>8}  {:>10}  {:<10}  FOLDER", "SIZE", "ENTRIES", "BUILT");
    for item in items {
        let built = match item.root {
            Some(_) => age(item.built),
            None => "unknown".to_string(),
        };
        println!(
            "{:>8}  {:>10}  {built:<10}  {}",
            bytes(item.size),
            grouped(item.entries),
            item.label()
        );
    }
    let total = items.iter().map(|item| item.size).sum();
    println!(
        "\n{}, {}.",
        counted(items.len() as u64, "index", "indexes"),
        bytes(total)
    );
    let lapsed = items.iter().filter(|item| item.lapsed()).count();
    if lapsed > 0 {
        println!(
            "{} can go. Run syk -A prune to remove them.",
            counted(lapsed as u64, "index", "indexes")
        );
    }
    ExitCode::SUCCESS
}

fn remove(store: &Path, items: &[&Held]) -> ExitCode {
    let gone: Vec<&&Held> = items.iter().filter(|item| item.remove()).collect();
    let freed = gone.iter().map(|item| item.size).sum();
    prune(store);
    println!(
        "Removed {} and freed {}.",
        counted(gone.len() as u64, "index", "indexes"),
        bytes(freed)
    );
    match gone.len() == items.len() {
        true => ExitCode::SUCCESS,
        false => ExitCode::from(2),
    }
}

fn forget_refusals(store: &Path) {
    let Ok(listing) = fs::read_dir(store) else {
        return;
    };
    for path in listing.flatten().map(|entry| entry.path()) {
        if path.extension().is_some_and(|kind| kind == "no") {
            let _ = fs::remove_file(path);
        }
    }
}

pub fn run(action: Action, paths: &[PathBuf]) -> ExitCode {
    let Some(store) = store() else {
        eprintln!("syk: there is no cache folder, because HOME is not set.");
        return ExitCode::from(2);
    };
    if action == Action::Prune && !paths.is_empty() {
        eprintln!("syk: -A prune looks at every index and takes no path.");
        return ExitCode::from(2);
    }
    let roots: Vec<PathBuf> = paths.iter().map(|path| real(path)).collect();
    let items: Vec<Held> = held(&store)
        .into_iter()
        .filter(|item| {
            roots.is_empty() || item.root.as_ref().is_some_and(|root| roots.contains(root))
        })
        .collect();
    match action {
        Action::List if items.is_empty() && !roots.is_empty() => {
            println!("No index for {}.", shown(&roots));
            ExitCode::from(1)
        }
        Action::List => list(&store, &items),
        Action::Clear if items.is_empty() && !roots.is_empty() => {
            println!("No index for {}.", shown(&roots));
            ExitCode::from(1)
        }
        Action::Clear => {
            if roots.is_empty() {
                forget_refusals(&store);
            }
            remove(&store, &items.iter().collect::<Vec<_>>())
        }
        Action::Prune => {
            let lapsed: Vec<&Held> = items.iter().filter(|item| item.lapsed()).collect();
            remove(&store, &lapsed)
        }
    }
}

fn shown(roots: &[PathBuf]) -> String {
    roots
        .iter()
        .map(|root| root.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_read_in_the_nearest_unit() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(1_500), "1.5 KB");
        assert_eq!(bytes(204_000_000), "204 MB");
    }

    #[test]
    fn only_known_actions_are_taken() {
        assert_eq!(Action::of(""), Ok(Action::List));
        assert_eq!(Action::of("clear"), Ok(Action::Clear));
        assert!(Action::of("wipe").is_err());
    }
}
