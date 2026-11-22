pub const EVERY: &str = "all";
const SHORTEST: usize = 3;

pub struct Preset {
    pub name: &'static str,
    pub active: bool,
    pub patterns: &'static [&'static str],
}

pub const PRESETS: [Preset; 6] = [
    Preset {
        name: "deps",
        active: true,
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
        active: true,
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
        active: false,
        patterns: &[
            "target", "dist", "build", "out", "_build", "coverage", "*.o", "*.class",
        ],
    },
    Preset {
        name: "lock",
        active: false,
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
        active: false,
        patterns: &["*.min.js", "*.min.css", "*.map"],
    },
    Preset {
        name: "media",
        active: false,
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
    pub active: bool,
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
            active: preset.active,
        })
        .collect()
}

fn unknown(groups: &[Group], name: &str) -> String {
    let names: Vec<&str> = groups.iter().map(|group| group.name.as_str()).collect();
    format!(
        "{name:?} is not a group. The groups are {}, and {EVERY} for every one of them.",
        names.join(", ")
    )
}

fn named(name: &str) -> bool {
    name != EVERY
        && !name.is_empty()
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

pub fn toggle(groups: &mut [Group], names: &str, active: bool) -> Result<(), String> {
    for name in names.split(',').filter(|name| !name.is_empty()) {
        if name == EVERY {
            groups.iter_mut().for_each(|group| group.active = active);
            continue;
        }
        let Some(at) = groups.iter().position(|group| group.name == name) else {
            return Err(unknown(groups, name));
        };
        groups[at].active = active;
    }
    Ok(())
}

pub fn define(groups: &mut Vec<Group>, name: &str, patterns: &str) -> Result<(), String> {
    let (name, extends) = match name.strip_suffix('+') {
        Some(name) => (name, true),
        None => (name, false),
    };
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
    match groups.iter_mut().find(|group| group.name == name) {
        Some(group) if extends => {
            group.patterns.extend(patterns);
            group.active = true;
        }
        Some(group) => {
            group.patterns = patterns;
            group.active = true;
        }
        None => groups.push(Group {
            name: name.to_string(),
            patterns,
            active: true,
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

    pub fn new(groups: &[Group]) -> Skips {
        let mut skips = Skips::none();
        let active = groups.iter().filter(|group| group.active);
        for pattern in active.flat_map(|group| &group.patterns) {
            match pattern.strip_prefix('*') {
                Some(suffix) if !suffix.is_empty() && plain(suffix) => {
                    skips.suffixes.push(suffix.as_bytes().into());
                }
                None if plain(pattern) => {
                    skips.starts[usize::from(pattern.as_bytes()[0])] = true;
                    skips.names.push(pattern.as_bytes().into());
                }
                _ => skips.globs.push(pattern.clone()),
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
        .filter(|group| group.active)
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

    fn active(groups: &[Group]) -> Vec<&str> {
        groups
            .iter()
            .filter(|group| group.active)
            .map(|group| group.name.as_str())
            .collect()
    }

    #[test]
    fn groups_are_switched_by_name() {
        let mut groups = presets();
        assert_eq!(active(&groups), ["deps", "cache"]);
        toggle(&mut groups, "lock,min", true).expect("known groups");
        toggle(&mut groups, "deps", false).expect("a known group");
        assert_eq!(active(&groups), ["cache", "lock", "min"]);
        toggle(&mut groups, EVERY, false).expect("every group");
        assert!(active(&groups).is_empty());
        let error = toggle(&mut groups, "depz", true).expect_err("an unknown group");
        assert!(error.contains("\"depz\" is not a group") && error.contains("deps, cache"));
    }

    #[test]
    fn groups_are_made_replaced_and_extended() {
        let mut groups = presets();
        define(&mut groups, "snapshots", "__snapshots__,*.snap").expect("a new group");
        define(&mut groups, "deps+", "vendor").expect("an extended group");
        define(&mut groups, "lock", "Cargo.lock").expect("a replaced group");
        assert_eq!(active(&groups), ["deps", "cache", "lock", "snapshots"]);
        let skips = Skips::new(&groups);
        for name in [
            "vendor",
            "node_modules",
            "__snapshots__",
            "a.snap",
            "Cargo.lock",
        ] {
            assert!(skips.matches(name.as_bytes()), "{name} is not skipped");
        }
        assert!(!skips.matches(b"yarn.lock"));
        assert!(define(&mut groups, "all", "x").is_err());
        assert!(define(&mut groups, "a b", "x").is_err());
        assert!(define(&mut groups, "empty", ",").is_err());
    }

    #[test]
    fn names_match_whole_and_suffixes_match_the_end() {
        let mut groups = presets();
        toggle(&mut groups, EVERY, true).expect("every group");
        define(&mut groups, "mine", "docs/generated,test_*,legacy/").expect("a new group");
        let skips = Skips::new(&groups);
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
        assert!(!Skips::none().by_name() && Skips::new(&presets()).by_name());
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
            assert_eq!(
                line.contains("skipped by default"),
                preset.active,
                "{} has the wrong default in the help",
                preset.name
            );
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
