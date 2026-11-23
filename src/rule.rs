use std::ffi::OsStr;
use std::fs::{self, File, FileType};
use std::io::{BufRead, BufReader};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use ignore::Match;
use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::overrides::Override;

use crate::group::{Skip, Skips};
use crate::query::Query;
use crate::walk::{excludes, excludes_file, skips};

#[derive(Default)]
pub struct Seen {
    ignore: bool,
    gitignore: bool,
    git: bool,
    jj: bool,
}

impl Seen {
    pub fn note(&mut self, name: &[u8]) {
        match name {
            b".ignore" => self.ignore = true,
            b".gitignore" => self.gitignore = true,
            b".git" => self.git = true,
            b".jj" => self.jj = true,
            _ => {}
        }
    }

    pub fn any(&self) -> bool {
        self.ignore || self.gitignore || self.git || self.jj
    }
}

pub struct Layer {
    parent: Option<Arc<Layer>>,
    ignore: Gitignore,
    gitignore: Gitignore,
    exclude: Gitignore,
    git: bool,
    any_git: bool,
    loud: bool,
}

fn matcher(dir: &Path, file: &Path, errors: &mut Vec<String>) -> Gitignore {
    let mut builder = GitignoreBuilder::new(dir);
    if file.exists()
        && let Some(error) = builder.add(file)
        && !error.is_io()
    {
        errors.push(error.to_string());
    }
    builder.build().unwrap_or_else(|error| {
        errors.push(error.to_string());
        Gitignore::empty()
    })
}

fn first_line(path: &Path, errors: &mut Vec<String>) -> Option<String> {
    match BufReader::new(File::open(path).ok()?).lines().next()? {
        Ok(line) => Some(line),
        Err(error) => {
            errors.push(format!("{}: {error}", path.display()));
            None
        }
    }
}

fn common(dir: &Path, kind: Option<FileType>, errors: &mut Vec<String>) -> Option<PathBuf> {
    let dot = dir.join(".git");
    if !kind.is_some_and(|kind| kind.is_file()) {
        return Some(dot);
    }
    if let Err(error) = File::open(&dot) {
        errors.push(format!("{}: {error}", dot.display()));
        return None;
    }
    let line = first_line(&dot, errors)?;
    let real = PathBuf::from(line.strip_prefix("gitdir: ")?);
    let line = first_line(&real.join("commondir"), errors)?;
    Some(match line.starts_with('.') {
        true => real.join(line),
        false => PathBuf::from(line),
    })
}

impl Layer {
    pub fn read(
        dir: &Path,
        seen: Option<&Seen>,
        parent: Option<Arc<Layer>>,
        above: bool,
        errors: &mut Vec<String>,
    ) -> Layer {
        let listed = |pick: fn(&Seen) -> bool| seen.is_none_or(pick);
        let empty = Gitignore::empty;
        let kind = match listed(|seen| seen.git) {
            true => fs::metadata(dir.join(".git"))
                .ok()
                .map(|found| found.file_type()),
            false => None,
        };
        let jj = listed(|seen| seen.jj) && dir.join(".jj").exists();
        let ignore = match listed(|seen| seen.ignore) {
            true => matcher(dir, &dir.join(".ignore"), errors),
            false => empty(),
        };
        let gitignore = match listed(|seen| seen.gitignore) {
            true => matcher(dir, &dir.join(".gitignore"), errors),
            false => empty(),
        };
        let exclude = match listed(|seen| seen.git).then(|| common(dir, kind, errors)) {
            Some(Some(git)) => matcher(dir, &git.join("info/exclude"), errors),
            _ => empty(),
        };
        let git = kind.is_some() || jj;
        let own = !ignore.is_empty() || !gitignore.is_empty() || !exclude.is_empty();
        Layer {
            any_git: git || parent.as_ref().map_or(above, |parent| parent.any_git),
            loud: own || parent.as_ref().is_some_and(|parent| parent.loud),
            parent,
            ignore,
            gitignore,
            exclude,
            git,
        }
    }
}

pub struct Base {
    root: PathBuf,
    real: PathBuf,
    above: Vec<Layer>,
    rules: bool,
    pub any_git: bool,
    pub failure: Option<String>,
}

impl Base {
    pub fn new(root: &Path, active: bool) -> Base {
        let mut base = Base {
            root: root.to_path_buf(),
            real: PathBuf::new(),
            above: Vec::new(),
            rules: false,
            any_git: false,
            failure: None,
        };
        let Some(real) = active.then(|| fs::canonicalize(root).ok()).flatten() else {
            return base;
        };
        let mut errors = Vec::new();
        let dirs: Vec<&Path> = real.ancestors().skip(1).collect();
        for dir in dirs.into_iter().rev() {
            base.above
                .push(Layer::read(dir, None, None, false, &mut errors));
        }
        base.above.reverse();
        base.rules = base.above.iter().any(|layer| layer.loud);
        base.any_git = base.above.iter().any(|layer| layer.git);
        base.failure = (!errors.is_empty()).then(|| errors.join("\n"));
        base.real = real;
        base
    }

    fn inner<'a>(&self, shown: &'a Path) -> &'a Path {
        if self.root.as_path() == Path::new(".") {
            return shown;
        }
        let root = self.root.as_os_str().as_bytes();
        let root = root.strip_prefix(b"./").unwrap_or(root);
        let bytes = shown.as_os_str().as_bytes();
        let rest = bytes.strip_prefix(root).unwrap_or(bytes);
        Path::new(OsStr::from_bytes(rest.strip_prefix(b"/").unwrap_or(rest)))
    }
}

fn global(here: &Path) -> Gitignore {
    let Some(path) = excludes_file().filter(|path| path.is_file()) else {
        return Gitignore::empty();
    };
    let mut builder = GitignoreBuilder::new(here);
    let _ = builder.add(path);
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

fn verdict<T>(found: Match<T>) -> Option<bool> {
    match found {
        Match::None => None,
        Match::Ignore(_) => Some(true),
        Match::Whitelist(_) => Some(false),
    }
}

pub struct Rules {
    hidden: bool,
    active: bool,
    git: bool,
    skips: Skips,
    excluded: Override,
    global: OnceLock<Gitignore>,
}

impl Rules {
    pub fn new(query: &Query) -> Rules {
        let hidden = query.walk.hidden || query.walk.all;
        let ignored = query.walk.no_ignore || query.walk.all;
        Rules {
            hidden: !hidden,
            active: !ignored,
            git: hidden && !query.walk.all,
            skips: skips(query, Skip::Whole),
            excluded: excludes(query).unwrap_or_else(|_| Override::empty()),
            global: OnceLock::new(),
        }
    }

    pub fn active(&self) -> bool {
        self.active
    }

    fn global(&self) -> &Gitignore {
        self.global.get_or_init(|| match std::env::current_dir() {
            Ok(here) => global(&here),
            Err(_) => Gitignore::empty(),
        })
    }

    fn ruled(
        &self,
        base: &Base,
        layer: Option<&Layer>,
        shown: &Path,
        folder: bool,
    ) -> Option<bool> {
        let any_git = layer.map_or(base.any_git, |layer| layer.any_git);
        let loud = layer.is_some_and(|layer| layer.loud)
            || base.rules
            || (any_git && !self.global().is_empty());
        if !self.active || !loud {
            return None;
        }
        let (mut ignored, mut tracked, mut excluded) = (None, None, None);
        let mut saw_git = false;
        let mut weigh = |layer: &Layer, path: &Path| {
            if ignored.is_none() {
                ignored = verdict(layer.ignore.matched(path, folder));
            }
            if any_git && !saw_git && tracked.is_none() {
                tracked = verdict(layer.gitignore.matched(path, folder));
            }
            if any_git && !saw_git && excluded.is_none() {
                excluded = verdict(layer.exclude.matched(path, folder));
            }
            saw_git |= layer.git;
        };
        let mut next = layer;
        while let Some(layer) = next {
            weigh(layer, shown);
            next = layer.parent.as_deref();
        }
        if base.rules {
            let full = base.real.join(base.inner(shown));
            for layer in &base.above {
                weigh(layer, &full);
            }
        }
        let global = match any_git {
            true => verdict(self.global().matched(shown, folder)),
            false => None,
        };
        ignored.or(tracked).or(excluded).or(global)
    }

    pub fn skip(
        &self,
        base: &Base,
        layer: Option<&Layer>,
        path: &[u8],
        name: &[u8],
        folder: bool,
    ) -> bool {
        let shown = Path::new(OsStr::from_bytes(path.strip_prefix(b"./").unwrap_or(path)));
        let chosen = match self.excluded.is_empty() {
            true => None,
            false => verdict(self.excluded.matched(shown, folder)),
        };
        let ruled = chosen.or_else(|| self.ruled(base, layer, shown, folder));
        let dropped = ruled.unwrap_or(self.hidden && name.starts_with(b"."));
        dropped || (self.git && name == b".git") || self.skips.matches(name)
    }
}
