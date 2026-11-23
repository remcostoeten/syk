use std::path::PathBuf;

use crate::group::Group;

pub struct Query {
    pub plan: Plan,
    pub roots: Vec<PathBuf>,
    pub filters: Filters,
    pub matching: Matching,
    pub walk: Walk,
    pub output: Output,
    pub expanded: Option<Expanded>,
    pub piped: bool,
    pub no_config: bool,
    pub defaults: Option<Defaults>,
}

pub struct Defaults {
    pub path: PathBuf,
    pub args: Vec<String>,
}

pub type Terms = Vec<String>;

pub struct Expanded {
    pub extension: String,
    pub files: usize,
}

pub enum Plan {
    Union {
        pattern: Option<Terms>,
        targets: Targets,
    },
    Narrow {
        dir: Option<Terms>,
        name: Option<Terms>,
        content: Option<Terms>,
    },
}

#[derive(Clone, Copy)]
pub struct Targets {
    pub dirs: bool,
    pub files: bool,
    pub content: bool,
}

#[derive(Clone, Default)]
pub struct Filters {
    pub extensions: Vec<String>,
    pub excludes: Vec<String>,
    pub groups: Vec<Group>,
    pub sizes: Vec<SizeFilter>,
    pub ages: Vec<AgeFilter>,
    pub kind: Option<Kind>,
}

impl Filters {
    pub fn files_only(&self) -> bool {
        !self.extensions.is_empty() || !self.sizes.is_empty() || !self.ages.is_empty()
    }
}

#[derive(Clone)]
pub struct SizeFilter {
    pub at_least: bool,
    pub bytes: u64,
    pub text: String,
}

#[derive(Clone)]
pub struct AgeFilter {
    pub older: bool,
    pub seconds: u64,
    pub text: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    File,
    Dir,
    Link,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Case {
    #[default]
    Smart,
    Insensitive,
    Sensitive,
}

#[derive(Clone, Copy, Default)]
pub struct Matching {
    pub case: Case,
    pub literal: bool,
    pub word: bool,
}

#[derive(Default)]
pub struct Walk {
    pub hidden: bool,
    pub no_ignore: bool,
    pub all: bool,
    pub follow: bool,
    pub depth: Option<usize>,
    pub threads: usize,
    pub sort: bool,
    pub index: bool,
    pub no_index: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    #[default]
    Auto,
    Plain,
    Pretty,
}

#[derive(Default)]
pub struct Output {
    pub list: bool,
    pub max: Option<usize>,
    pub context: usize,
    pub format: Format,
    pub explain: bool,
    pub exact: bool,
    pub rank: Option<bool>,
    pub best: bool,
    pub why: bool,
    pub count: bool,
    pub stats: bool,
    pub json: bool,
    pub null: bool,
    pub no_links: bool,
    pub open: bool,
    pub exec: Option<String>,
    pub browse: bool,
}

impl Output {
    pub fn acts(&self) -> bool {
        self.open || self.exec.is_some()
    }
}
