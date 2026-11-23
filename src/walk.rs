use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ignore::overrides::{Override, OverrideBuilder};
use ignore::{DirEntry, WalkBuilder};

use crate::check::synthetic;
use crate::group::{Skip, Skips};
use crate::query::Query;

const WORKERS: usize = 12;
const KERNEL: [&str; 2] = ["proc", "sys"];

pub fn kernel(real: &Path) -> Vec<&'static str> {
    let top = Path::new("/");
    match real == top {
        true => KERNEL
            .into_iter()
            .filter(|name| synthetic(&top.join(name)))
            .collect(),
        false => Vec::new(),
    }
}

pub struct Bars {
    git: bool,
    skips: Skips,
    kernel: Vec<PathBuf>,
}

impl Bars {
    pub fn new(query: &Query) -> Bars {
        let hidden = query.walk.hidden || query.walk.all;
        let ignored = query.walk.no_ignore || query.walk.all;
        let made = |root: &PathBuf| -> Vec<PathBuf> {
            let real = fs::canonicalize(root).unwrap_or_default();
            let names = kernel(&real).into_iter();
            names.map(|name| root.join(name)).collect()
        };
        Bars {
            git: hidden && !query.walk.all,
            skips: skips(query, Skip::Whole),
            kernel: match ignored {
                true => Vec::new(),
                false => query.roots.iter().flat_map(made).collect(),
            },
        }
    }

    fn any(&self) -> bool {
        self.git || self.skips.by_name() || !self.kernel.is_empty()
    }

    pub fn bar(&self, entry: &DirEntry) -> bool {
        let name = entry.file_name().as_bytes();
        let named = (self.git && name == b".git") || self.skips.matches(name);
        let made = entry.depth() == 1 && self.kernel.iter().any(|path| path == entry.path());
        entry.depth() > 0 && (named || made)
    }
}

pub fn excludes(query: &Query) -> Result<Override, String> {
    let root = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let mut builder = OverrideBuilder::new(root);
    for exclude in &query.filters.excludes {
        let mut globs = vec![format!("!{exclude}")];
        if exclude.starts_with('.') && !exclude.contains(['/', '*', '?', '[']) {
            globs.push(format!("!*{exclude}"));
        }
        for glob in globs {
            builder
                .add(&glob)
                .map_err(|error| format!("-E {exclude} is not a usable pattern: {error}"))?;
        }
    }
    for glob in skips(query, Skip::Whole).globs {
        builder
            .add(&format!("!{glob}"))
            .map_err(|error| format!("-g {glob} is not a usable pattern: {error}"))?;
    }
    builder.build().map_err(|error| error.to_string())
}

pub fn skips(query: &Query, skip: Skip) -> Skips {
    if query.walk.no_ignore || query.walk.all {
        Skips::none()
    } else {
        Skips::new(&query.filters.groups, skip)
    }
}

pub fn walker(query: &Query) -> WalkBuilder {
    let hidden = query.walk.hidden || query.walk.all;
    let ignored = query.walk.no_ignore || query.walk.all;
    let mut builder = WalkBuilder::new(&query.roots[0]);
    for root in &query.roots[1..] {
        builder.add(root);
    }
    builder
        .hidden(!hidden)
        .ignore(!ignored)
        .git_ignore(!ignored)
        .git_global(!ignored)
        .git_exclude(!ignored)
        .parents(!ignored)
        .follow_links(query.walk.follow)
        .max_depth(query.walk.depth)
        .threads(threads(query));
    let bars = Bars::new(query);
    if bars.any() {
        builder.filter_entry(move |entry| !bars.bar(entry));
    }
    if let Ok(excluded) = excludes(query)
        && !excluded.is_empty()
    {
        builder.overrides(excluded);
    }
    builder
}

pub fn threads(query: &Query) -> usize {
    static CORES: OnceLock<usize> = OnceLock::new();
    match query.walk.threads {
        0 => *CORES.get_or_init(|| {
            std::thread::available_parallelism().map_or(1, |cores| cores.get().min(WORKERS))
        }),
        chosen => chosen,
    }
}

fn mentioned(path: PathBuf) -> bool {
    let word = b"excludesfile";
    fs::read(path).is_ok_and(|text| {
        text.windows(word.len())
            .any(|part| part.eq_ignore_ascii_case(word))
    })
}

pub fn excludes_file() -> Option<PathBuf> {
    let home = std::env::home_dir();
    let settings = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|folder| !folder.as_os_str().is_empty())
        .or_else(|| home.as_ref().map(|home| home.join(".config")));
    let named = home.is_some_and(|home| mentioned(home.join(".gitconfig")))
        || settings
            .as_ref()
            .is_some_and(|settings| mentioned(settings.join("git/config")));
    // The crate compiles a regular expression to read the setting, which only pays when it is set
    match named {
        true => ignore::gitignore::gitconfig_excludes_path(),
        false => settings.map(|settings| settings.join("git/ignore")),
    }
}
