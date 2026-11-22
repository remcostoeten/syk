pub const EVERY: &str = "all";
const SHORTEST: usize = 3;
const CONTENTS: [&str; 2] = [":content", ":contents"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Skip {
    Off,
    Contents,
    Whole,
}

pub struct Preset {
    pub name: &'static str,
    pub skip: Skip,
    pub patterns: &'static [&'static str],
}

pub const PRESETS: [Preset; 6] = [
    Preset {
        name: "deps",
        skip: Skip::Whole,
        patterns: &[
            "node_modules",
            "bower_components",
            "jspm_packages",
            "site-packages",
            "venv",
            ".venv",
            "Pods",
            ".pnpm-store",
        ],
    },
    Preset {
        name: "cache",
        skip: Skip::Whole,
        patterns: &[
            "__pycache__",
            "*.pyc",
            "*.pyo",
            ".cache",
            ".pytest_cache",
            ".mypy_cache",
            ".ruff_cache",
            ".tox",
            ".gradle",
            ".turbo",
            ".parcel-cache",
            ".sass-cache",
            ".eslintcache",
            ".next",
            ".nuxt",
            ".svelte-kit",
            ".DS_Store",
            "Thumbs.db",
        ],
    },
    Preset {
        name: "build",
        skip: Skip::Off,
        patterns: &[
            "target", "dist", "build", "out", "_build", "coverage", "*.o", "*.class",
        ],
    },
    Preset {
        name: "lock",
        skip: Skip::Contents,
        patterns: &[
            "package-lock.json",
            "yarn.lock",
            "pnpm-lock.yaml",
            "bun.lock",
            "bun.lockb",
            "Cargo.lock",
            "composer.lock",
            "Gemfile.lock",
            "poetry.lock",
            "uv.lock",
            "Pipfile.lock",
            "go.sum",
            "flake.lock",
            "mix.lock",
            "pubspec.lock",
            "Podfile.lock",
        ],
    },
    Preset {
        name: "min",
        skip: Skip::Off,
        patterns: &["*.min.js", "*.min.css", "*.map"],
    },
    Preset {
        name: "media",
        skip: Skip::Off,
        patterns: &[
            "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.avif", "*.ico", "*.bmp", "*.svg",
            "*.mp3", "*.wav", "*.ogg", "*.flac", "*.mp4", "*.mov", "*.mkv", "*.webm", "*.avi",
            "*.woff", "*.woff2", "*.ttf", "*.otf", "*.eot",
        ],
    },
];

#[derive(Clone)]
pub struct Group {
    pub name: String,
    pub patterns: Vec<String>,
    pub skip: Skip,
}

pub fn presets() -> Vec<Group> {
    PRESETS
        .iter()
        .map(|preset| Group {
            name: preset.name.to_string(),
            patterns: preset
                .patterns
                .iter()
                .map(|text| text.to_string())
                .collect(),
            skip: preset.skip,
        })
        .collect()
}

fn unknown(groups: &[Group], name: &str) -> String {
    let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
    let pattern = if name.contains(['*', '?', '[', '/', '.']) {
        format!(" To leave out names by pattern, use -E {name:?}.")
    } else {
        String::new()
    };
    format!(
        "{name:?} is not a group. The groups are {}, and {EVERY} for every one of them.{pattern}",
        names.join(", ")
    )
}

fn named(name: &str) -> bool {
    !name.eq_ignore_ascii_case(EVERY)
        && !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn extent(name: &str) -> (&str, Option<Skip>) {
    CONTENTS
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .map_or((name, None), |name| (name, Some(Skip::Contents)))
}

fn set(groups: &mut [Group], name: &str, skip: Skip) -> Result<(), String> {
    if name.eq_ignore_ascii_case(EVERY) {
        groups.iter_mut().for_each(|group| group.skip = skip);
        return Ok(());
    }
    let Some(at) = groups
        .iter()
        .position(|group| group.name.eq_ignore_ascii_case(name))
    else {
        return Err(unknown(groups, name));
    };
    groups[at].skip = skip;
    Ok(())
}

pub fn skip(groups: &mut [Group], names: &str) -> Result<(), String> {
    for name in names.split(',').filter(|name| !name.is_empty()) {
        let (name, chosen) = extent(name);
        set(groups, name, chosen.unwrap_or(Skip::Whole))?;
    }
    Ok(())
}

pub fn search(groups: &mut [Group], names: &str) -> Result<(), String> {
    for name in names.split(',').filter(|name| !name.is_empty()) {
        set(groups, name, Skip::Off)?;
    }
    Ok(())
}

pub fn define(groups: &mut Vec<Group>, name: &str, patterns: &str) -> Result<(), String> {
    let (name, extends) = match name.strip_suffix('+') {
        Some(name) => (name, true),
        None => (name, false),
    };
    let (name, chosen) = extent(name);
    if !named(name) {
        return Err(format!(
            "{name:?} cannot name a group. Use letters, digits, - and _, as in -g snapshots=__snapshots__,*.snap."
        ));
    }
    let patterns: Vec<String> = patterns
        .split(',')
        .filter(|pattern| !pattern.is_empty())
        .map(str::to_string)
        .collect();
    if patterns.is_empty() {
        return Err(format!(
            "The group {name} needs at least one name after the = sign."
        ));
    }
    match groups
        .iter_mut()
        .find(|group| group.name.eq_ignore_ascii_case(name))
    {
        Some(group) if extends => {
            group.patterns.extend(patterns);
            group.skip = match (chosen, group.skip) {
                (Some(chosen), _) => chosen,
                (None, Skip::Off) => Skip::Whole,
                (None, kept) => kept,
            };
        }
        Some(group) => {
            group.patterns = patterns;
            group.skip = chosen.unwrap_or(Skip::Whole);
        }
        None => groups.push(Group {
            name: name.to_string(),
            patterns,
            skip: chosen.unwrap_or(Skip::Whole),
        }),
    }
    Ok(())
}

pub fn ends_with(name: &[u8], suffix: &[u8]) -> bool {
    name.len() >= suffix.len() && name[name.len() - suffix.len()..].eq_ignore_ascii_case(suffix)
}

fn plain(text: &str) -> bool {
    !text.contains(['/', '*', '?', '['])
}

fn ending(pattern: &str) -> Option<&str> {
    pattern
        .strip_prefix('*')
        .filter(|suffix| !suffix.is_empty() && plain(suffix))
}

pub fn check(groups: &[Group]) -> Result<(), String> {
    for group in groups.iter().filter(|group| group.skip == Skip::Contents) {
        if let Some(pattern) = group
            .patterns
            .iter()
            .find(|pattern| ending(pattern).is_none() && !plain(pattern))
        {
            return Err(format!(
                "{pattern} cannot be in the group {}, because only its contents are skipped. That works for whole names and for endings such as *.snap.",
                group.name
            ));
        }
    }
    Ok(())
}

pub struct Skips {
    names: Vec<Box<[u8]>>,
    starts: [bool; 256],
    suffixes: Vec<Box<[u8]>>,
    pub globs: Vec<String>,
}

impl Skips {
    pub fn none() -> Skips {
        Skips {
            names: Vec::new(),
            starts: [false; 256],
            suffixes: Vec::new(),
            globs: Vec::new(),
        }
    }

    pub fn new(groups: &[Group], skip: Skip) -> Skips {
        let mut skips = Skips::none();
        let chosen = groups.iter().filter(|group| group.skip == skip);
        for pattern in chosen.flat_map(|group| &group.patterns) {
            match ending(pattern) {
                Some(suffix) => skips.suffixes.push(suffix.as_bytes().into()),
                None if plain(pattern) => {
                    skips.starts[usize::from(pattern.as_bytes()[0])] = true;
                    skips.names.push(pattern.as_bytes().into());
                }
                None => skips.globs.push(pattern.clone()),
            }
        }
        skips
    }

    pub fn by_name(&self) -> bool {
        !self.names.is_empty() || !self.suffixes.is_empty()
    }

    pub fn matches(&self, name: &[u8]) -> bool {
        let Some(first) = name.first() else {
            return false;
        };
        (self.starts[usize::from(*first)] && self.names.iter().any(|known| **known == *name))
            || self.suffixes.iter().any(|suffix| ends_with(name, suffix))
    }
}

pub fn covering<'a>(groups: &'a [Group], term: &str) -> Option<(&'a Group, &'a str)> {
    if term.len() < SHORTEST {
        return None;
    }
    let term = term.to_ascii_lowercase();
    groups
        .iter()
        .filter(|group| group.skip == Skip::Whole)
        .find_map(|group| {
            let pattern = group.patterns.iter().find(|pattern| {
                let literal = pattern.trim_start_matches('*').to_ascii_lowercase();
                literal.contains(&term) || term.contains(&literal)
            })?;
            Some((group, pattern.as_str()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flags::{self, Id};

    fn skipped(groups: &[Group], skip: Skip) -> Vec<&str> {
        groups
            .iter()
            .filter(|group| group.skip == skip)
            .map(|group| group.name.as_str())
            .collect()
    }

    #[test]
    fn groups_are_switched_by_name() {
        let mut groups = presets();
        assert_eq!(skipped(&groups, Skip::Whole), ["deps", "cache"]);
        assert_eq!(skipped(&groups, Skip::Contents), ["lock"]);
        skip(&mut groups, "lock,min:content,media:contents").expect("known groups");
        search(&mut groups, "deps").expect("a known group");
        assert_eq!(skipped(&groups, Skip::Whole), ["cache", "lock"]);
        assert_eq!(skipped(&groups, Skip::Contents), ["min", "media"]);
        search(&mut groups, EVERY).expect("every group");
        assert_eq!(skipped(&groups, Skip::Off).len(), PRESETS.len());
        skip(&mut groups, "all:content").expect("every group");
        assert_eq!(skipped(&groups, Skip::Contents).len(), PRESETS.len());
        let error = skip(&mut groups, "depz").expect_err("an unknown group");
        assert!(error.contains("\"depz\" is not a group") && error.contains("deps, cache"));
        assert!(!error.contains("-E"));
        let error = skip(&mut groups, "*.ts").expect_err("a pattern");
        assert!(error.ends_with("use -E \"*.ts\"."));
        search(&mut groups, "ALL").expect("every group");
        skip(&mut groups, "Lock,DEPS:content").expect("known groups");
        assert_eq!(skipped(&groups, Skip::Whole), ["lock"]);
        assert_eq!(skipped(&groups, Skip::Contents), ["deps"]);
        assert!(search(&mut groups, "lock:content").is_err());
    }

    #[test]
    fn groups_are_made_replaced_and_extended() {
        let mut groups = presets();
        define(&mut groups, "snapshots", "__snapshots__,*.snap").expect("a new group");
        define(&mut groups, "deps+", "vendor").expect("an extended group");
        define(&mut groups, "lock+", "deno.lock").expect("an extended group");
        define(&mut groups, "min", "*.min.js").expect("a replaced group");
        define(&mut groups, "fixtures:content", "fixtures,*.snap").expect("a new group");
        assert_eq!(
            skipped(&groups, Skip::Whole),
            ["deps", "cache", "min", "snapshots"]
        );
        assert_eq!(skipped(&groups, Skip::Contents), ["lock", "fixtures"]);
        let whole = Skips::new(&groups, Skip::Whole);
        for name in [
            "vendor",
            "node_modules",
            "__snapshots__",
            "a.snap",
            "a.min.js",
        ] {
            assert!(whole.matches(name.as_bytes()), "{name} is not skipped");
        }
        assert!(!whole.matches(b"a.map") && !whole.matches(b"deno.lock"));
        let contents = Skips::new(&groups, Skip::Contents);
        for name in ["deno.lock", "Cargo.lock", "fixtures", "b.snap"] {
            assert!(contents.matches(name.as_bytes()), "{name} is read");
        }
        assert!(define(&mut groups, "all", "x").is_err());
        assert!(define(&mut groups, "a b", "x").is_err());
        assert!(define(&mut groups, "empty", ",").is_err());
    }

    #[test]
    fn names_match_whole_and_suffixes_match_the_end() {
        let mut groups = presets();
        skip(&mut groups, EVERY).expect("every group");
        define(&mut groups, "mine", "docs/generated,test_*,legacy/").expect("a new group");
        let skips = Skips::new(&groups, Skip::Whole);
        for name in [
            "node_modules",
            "main.pyc",
            "PHOTO.JPG",
            "app.min.js",
            "go.sum",
        ] {
            assert!(skips.matches(name.as_bytes()), "{name} is not skipped");
        }
        for name in [
            "node_modules2",
            "my_node_modules",
            "pyc",
            "",
            "distance",
            "lock",
        ] {
            assert!(!skips.matches(name.as_bytes()), "{name} is skipped");
        }
        assert_eq!(skips.globs, ["docs/generated", "test_*", "legacy/"]);
        assert!(!Skips::none().by_name() && Skips::new(&presets(), Skip::Whole).by_name());
    }

    #[test]
    fn a_group_with_only_its_contents_skipped_takes_names_and_endings() {
        let mut groups = presets();
        assert!(check(&groups).is_ok());
        define(&mut groups, "mine:content", "fixtures,*.snap").expect("a new group");
        assert!(check(&groups).is_ok());
        define(&mut groups, "mine+", "docs/generated").expect("an extended group");
        let error = check(&groups).expect_err("a path in a contents group");
        assert!(error.contains("docs/generated cannot be in the group mine"));
        skip(&mut groups, "mine").expect("a known group");
        assert!(check(&groups).is_ok());
    }

    #[test]
    fn a_term_that_a_skipped_group_covers_is_noticed() {
        let groups = presets();
        let found = covering(&groups, "node_mod").expect("a covered term");
        assert_eq!((found.0.name.as_str(), found.1), ("deps", "node_modules"));
        assert_eq!(covering(&groups, "Main.PYC").expect("a suffix").1, "*.pyc");
        assert!(covering(&groups, "lock").is_none());
        assert!(covering(&groups, "py").is_none());
    }

    #[test]
    fn the_help_page_lists_every_group_and_name() {
        let detail = flags::by_id(Id::Ignore).detail;
        for preset in &PRESETS {
            let line = detail
                .lines()
                .find(|line| line.starts_with(preset.name))
                .unwrap_or_else(|| panic!("the group {} is not in the help", preset.name));
            let stated = if line.contains(", skipped by default:") {
                Skip::Whole
            } else if line.contains(", contents not searched by default:") {
                Skip::Contents
            } else {
                Skip::Off
            };
            assert_eq!(stated, preset.skip, "{} has the wrong default", preset.name);
            for pattern in preset.patterns {
                assert!(
                    line.contains(pattern),
                    "{pattern} of {} is not in the help",
                    preset.name
                );
            }
        }
    }
}
