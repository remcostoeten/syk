#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Id {
    Name,
    Dir,
    Content,
    Ext,
    Exclude,
    Size,
    Modified,
    Type,
    Depth,
    IgnoreCase,
    CaseSensitive,
    Literal,
    Word,
    Exact,
    Hidden,
    NoIgnore,
    All,
    Ignore,
    Unignore,
    Index,
    NoIndex,
    Indexes,
    Follow,
    List,
    Max,
    Context,
    Sort,
    Rank,
    NoRank,
    Best,
    Why,
    Picks,
    Threads,
    Plain,
    Pretty,
    Explain,
    Help,
    Version,
    Count,
    Stats,
    Json,
    Null,
    NoLinks,
    Open,
    Exec,
    NoConfig,
    Init,
    Setup,
    Browse,
    NoPager,
    NoBanner,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Target,
    Filter,
    Matching,
    Walk,
    Output,
    Ranking,
    Action,
    Other,
}

impl Group {
    pub const ALL: [Group; 8] = [
        Group::Target,
        Group::Filter,
        Group::Matching,
        Group::Walk,
        Group::Output,
        Group::Ranking,
        Group::Action,
        Group::Other,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::Target => "WHAT TO SEARCH",
            Group::Filter => "FILTERS",
            Group::Matching => "MATCHING",
            Group::Walk => "WHERE TO LOOK",
            Group::Output => "OUTPUT",
            Group::Ranking => "RANKING",
            Group::Action => "ACTIONS",
            Group::Other => "OTHER",
        }
    }
}

#[derive(Clone, Copy)]
pub enum Arity {
    Switch,
    Optional(&'static str),
    Required(&'static str),
}

pub struct Flag {
    pub id: Id,
    pub short: char,
    pub long: &'static str,
    pub aliases: &'static [&'static str],
    pub arity: Arity,
    pub group: Group,
    pub summary: &'static str,
    pub detail: &'static str,
    pub examples: &'static [(&'static str, &'static str)],
    pub related: &'static [Id],
}

impl Flag {
    pub fn label(&self) -> String {
        match self.arity {
            Arity::Switch => format!("-{}, --{}", self.short, self.long),
            Arity::Optional(value) => format!("-{}, --{} [{}]", self.short, self.long, value),
            Arity::Required(value) => format!("-{}, --{} {}", self.short, self.long, value),
        }
    }
}

pub fn by_short(short: char) -> Option<&'static Flag> {
    FLAGS.iter().find(|flag| flag.short == short)
}

pub fn by_long(long: &str) -> Option<&'static Flag> {
    FLAGS
        .iter()
        .find(|flag| flag.long == long || flag.aliases.contains(&long))
}

pub fn by_id(id: Id) -> &'static Flag {
    FLAGS
        .iter()
        .find(|flag| flag.id == id)
        .expect("every id has a flag")
}

pub fn closest(long: &str) -> Option<&'static Flag> {
    FLAGS
        .iter()
        .flat_map(|flag| {
            std::iter::once(flag.long)
                .chain(flag.aliases.iter().copied())
                .map(move |name| (distance(long, name), name, flag))
        })
        .filter(|(distance, name, _)| *distance <= 2.max(name.len() / 3))
        .filter(|(distance, name, _)| *distance < long.chars().count().min(name.len()))
        .min_by_key(|(distance, _, _)| *distance)
        .map(|(_, _, flag)| flag)
}

fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut row: Vec<usize> = (0..=right.len()).collect();
    for (i, a) in left.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, b) in right.iter().enumerate() {
            let substitute = diagonal + usize::from(a != *b);
            diagonal = row[j + 1];
            row[j + 1] = substitute.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[right.len()]
}

pub static FLAGS: [Flag; 51] = [
    Flag {
        id: Id::Name,
        short: 'n',
        long: "name",
        aliases: &["names", "iname"],
        arity: Arity::Optional("PATTERN"),
        group: Group::Target,
        summary: "Search the names of files and folders.",
        detail: "Matches PATTERN against the last part of each path: auth.ts for src/routes/auth.ts, and routes for the folder src/routes. File contents are not read, so this is the cheapest kind of search.\n\nWithout PATTERN it lists every file and folder, which is useful together with filters.\n\nCombined with -c and a second pattern, -n selects which files are read: only files whose name matches are searched for the content pattern. A bare pattern next to -n PATTERN is taken as that content pattern.",
        examples: &[
            ("syk -n config", "files and folders with config in the name"),
            ("syk -n", "every file and folder below the current folder"),
            ("syk -n .ts +100kb", "TypeScript files of 100 KB or more"),
            ("syk -n test -t file", "files only, no folders"),
            (
                "syk -n route -c auth",
                "lines matching auth in files named *route*",
            ),
            (
                "syk -n route auth",
                "the same, with the content pattern bare",
            ),
        ],
        related: &[Id::Dir, Id::Content, Id::Type],
    },
    Flag {
        id: Id::Dir,
        short: 'd',
        long: "dir",
        aliases: &["directory", "directories", "dirs", "folder", "folders"],
        arity: Arity::Optional("PATTERN"),
        group: Group::Target,
        summary: "Search folder names.",
        detail: "Matches PATTERN against the name of each folder and prints the folders that match, with a trailing slash. Files are not reported. Without PATTERN it lists every folder.\n\nCombined with -n or -c and a second pattern, -d becomes a scope: only files below a folder whose name matches are considered. That folder can be any number of levels above the file.\n\nFolders that are ignored or hidden are never entered, so they cannot match unless -I, -H or -a is given.",
        examples: &[
            ("syk -d test", "folders with test in the name"),
            ("syk -d", "every folder"),
            (
                "syk -d api -c auth",
                "lines matching auth below any folder named *api*",
            ),
            (
                "syk -d components -n button",
                "names matching button below any *components* folder",
            ),
            (
                "syk -d node_modules -I -D 3",
                "ignored folders too, at most 3 levels deep",
            ),
        ],
        related: &[Id::Name, Id::Content, Id::Depth],
    },
    Flag {
        id: Id::Content,
        short: 'c',
        long: "content",
        aliases: &["contents", "text", "grep"],
        arity: Arity::Optional("PATTERN"),
        group: Group::Target,
        summary: "Search the lines inside files.",
        detail: "Reads each file and prints the lines that match PATTERN, with their line numbers. File and folder names are not matched. A pattern is required.\n\nFiles that contain a NUL byte are treated as binary and skipped. Ignored and hidden files are skipped unless -I, -H or -a is given.\n\nCombined with -n or -d and a second pattern, only the files those select are read.\n\nA pattern that looks like a flag, a filter or a path can be attached with an equals sign, as in -c=-v or --content=.env.",
        examples: &[
            ("syk -c TODO", "lines that contain TODO"),
            (
                "syk -c 'fn main' .rs",
                "the same kind of search, in Rust files only",
            ),
            ("syk -c auth src/", "below src/ only"),
            (
                "syk -c auth -l",
                "only the paths of files that contain auth",
            ),
            ("syk -c auth -C 2", "two lines of context around each hit"),
            ("syk -c=-v", "a pattern that starts with a dash"),
        ],
        related: &[Id::Name, Id::Dir, Id::List, Id::Context],
    },
    Flag {
        id: Id::Ext,
        short: 'e',
        long: "ext",
        aliases: &["extension", "extensions"],
        arity: Arity::Required("EXT"),
        group: Group::Filter,
        summary: "Only files with this extension.",
        detail: "Keeps files whose name ends in .EXT, ignoring case. Give several extensions separated by commas, or repeat the flag. A file has to match one of them. Compound extensions such as d.ts and tar.gz work.\n\nThe shorthand is a bare token that starts with a dot or with *. such as .ts or '*.ts'. Keep the quotes around the second form: unquoted, the shell replaces *.ts with the matching file names in the current folder, or stops with an error when there are none. A token like that is read as a path instead when it exists on disk, for example .github.\n\nWhile an extension filter is active, folders are not reported.",
        examples: &[
            ("syk foo .ts", "foo in TypeScript files"),
            ("syk foo '*.ts'", "the same, written as a quoted wildcard"),
            ("syk foo -e ts,tsx", "the same for two extensions"),
            ("syk .md", "list every Markdown file"),
            ("syk -c import .d.ts", "a compound extension"),
        ],
        related: &[Id::Type, Id::Size],
    },
    Flag {
        id: Id::Exclude,
        short: 'E',
        long: "exclude",
        aliases: &["not", "skip", "without"],
        arity: Arity::Required("NAME"),
        group: Group::Filter,
        summary: "Leave out files and folders by name or pattern.",
        detail: "Skips every entry that matches, and does not enter a folder that matches. Give several separated by commas, or repeat the flag.\n\nA plain word is a whole name at any depth: -E dist skips every file and folder called dist, but not distance or dist.js. Wildcards work when quoted: '*.min.js' or 'test_*'. A value that starts with a dot is also taken as an extension, so -E .log skips every .log file. A value with a slash inside it is a path from the current folder: -E src/generated. A trailing slash limits it to folders: -E build/.\n\nThe rules are the ones .gitignore uses, and upper and lower case must match. Excludes also apply to the folders ranking knows from zoxide and from earlier picks.",
        examples: &[
            ("syk foo -E dist", "skip anything called dist"),
            ("syk foo -E test,docs", "skip two names"),
            ("syk -c TODO -E '*.min.js'", "skip minified files"),
            ("syk -n config -E .json", "skip an extension"),
            (
                "syk foo -I -E node_modules",
                "search ignored files, except node_modules",
            ),
        ],
        related: &[Id::Ext, Id::Ignore, Id::NoIgnore, Id::Depth],
    },
    Flag {
        id: Id::Size,
        short: 'S',
        long: "size",
        aliases: &[],
        arity: Arity::Required("SIZE"),
        group: Group::Filter,
        summary: "Only files of at least, or at most, this size.",
        detail: "+SIZE keeps files of SIZE or larger. -SIZE keeps files of SIZE or smaller. A size without a sign means +SIZE. Units are b, kb, mb, gb and tb, counted in steps of 1024. No unit means bytes. Decimals are allowed.\n\nThe shorthand is a bare token with a sign and a unit: +10kb or -1mb. Give two of them to describe a range.\n\nA size filter costs one extra system call per file, made only for files that already passed the name and extension checks. While a size filter is active, folders are not reported.",
        examples: &[
            ("syk +100mb", "list files of 100 MB or more"),
            ("syk foo +1kb -1mb", "foo in files between 1 KB and 1 MB"),
            ("syk -S 1.5gb", "list files of 1.5 GB or more"),
            ("syk -c password -10kb", "search small files only"),
        ],
        related: &[Id::Modified, Id::Ext],
    },
    Flag {
        id: Id::Modified,
        short: 'M',
        long: "modified",
        aliases: &["changed", "mtime", "age"],
        arity: Arity::Required("AGE"),
        group: Group::Filter,
        summary: "Only files older, or newer, than this age.",
        detail: "+AGE keeps files last modified more than AGE ago. -AGE keeps files modified within the last AGE. An age without a sign means -AGE. Units are s, min, h, d, w, mo and y. A month counts as 30 days and a year as 365.\n\nThe shorthand is a bare token with a sign and a unit: -2d or +1y.\n\nAn age filter costs one extra system call per file, made only for files that already passed the name and extension checks. While an age filter is active, folders are not reported.",
        examples: &[
            ("syk -2d", "list files modified in the last 2 days"),
            ("syk foo +1y", "foo in files untouched for over a year"),
            ("syk .log +30d", "log files older than 30 days"),
            (
                "syk -c TODO -M 90min",
                "TODO in files changed in the last 90 minutes",
            ),
        ],
        related: &[Id::Size, Id::Ext],
    },
    Flag {
        id: Id::Type,
        short: 't',
        long: "type",
        aliases: &["kind"],
        arity: Arity::Required("KIND"),
        group: Group::Filter,
        summary: "Only entries of this kind: file, dir or link.",
        detail: "file keeps regular files, dir keeps folders and link keeps symbolic links. The short forms f, d and l work, and so do folder and symlink.\n\nContents are only ever read from regular files, whatever this is set to.",
        examples: &[
            ("syk -n test -t file", "files named *test*, no folders"),
            ("syk -t link", "list every symbolic link"),
            ("syk foo -t dir", "the same as syk -d foo"),
        ],
        related: &[Id::Name, Id::Dir, Id::Follow],
    },
    Flag {
        id: Id::Depth,
        short: 'D',
        long: "depth",
        aliases: &["max-depth", "maxdepth"],
        arity: Arity::Required("N"),
        group: Group::Filter,
        summary: "Go at most N folders deep.",
        detail: "Entries directly inside a search path are at depth 1. With -D 1 only those are looked at and no folder is entered.",
        examples: &[
            ("syk -n -D 1", "what is directly in this folder"),
            ("syk foo -D 3", "foo, at most 3 levels down"),
        ],
        related: &[Id::Dir],
    },
    Flag {
        id: Id::IgnoreCase,
        short: 'i',
        long: "ignore-case",
        aliases: &["insensitive"],
        arity: Arity::Switch,
        group: Group::Matching,
        summary: "Treat upper and lower case as the same.",
        detail: "By default the search is smart about case: a pattern written entirely in lower case matches any case, and a pattern with at least one upper case letter matches exactly. -i makes every pattern match any case.",
        examples: &[("syk -i README", "matches readme, Readme and README")],
        related: &[Id::CaseSensitive],
    },
    Flag {
        id: Id::CaseSensitive,
        short: 's',
        long: "case-sensitive",
        aliases: &["sensitive"],
        arity: Arity::Switch,
        group: Group::Matching,
        summary: "Match upper and lower case exactly.",
        detail: "Turns smart case off. A lower case pattern then matches lower case text only.",
        examples: &[("syk -s todo", "matches todo, not TODO")],
        related: &[Id::IgnoreCase],
    },
    Flag {
        id: Id::Literal,
        short: 'F',
        long: "literal",
        aliases: &["fixed", "fixed-strings"],
        arity: Arity::Switch,
        group: Group::Matching,
        summary: "Search for the pattern as plain text.",
        detail: "Patterns are regular expressions by default, in the syntax of the Rust regex crate. Characters such as . ( [ * + ? have a special meaning there. -F switches that off, so every character stands for itself.\n\nIt is rarely needed. A pattern that is not a valid regular expression, such as foo( , is searched as plain text automatically, and one that only makes sense as a wildcard, such as *_test.go, is matched as a wildcard against names. -x shows which reading was used.\n\nUse -F when a pattern is valid as a regular expression but you mean it literally, such as a.b or price$.",
        examples: &[
            ("syk -F 'a.b'", "the three characters a.b"),
            ("syk -F '$HOME' .sh", "a dollar sign followed by HOME"),
        ],
        related: &[Id::Word, Id::Explain],
    },
    Flag {
        id: Id::Word,
        short: 'w',
        long: "word",
        aliases: &["word-regexp", "words"],
        arity: Arity::Switch,
        group: Group::Matching,
        summary: "Match whole words only.",
        detail: "The match must have a word boundary on both sides. A word character is a letter, a digit or an underscore.",
        examples: &[(
            "syk -w log",
            "matches log( and my log, not logger or catalog",
        )],
        related: &[Id::Literal],
    },
    Flag {
        id: Id::Exact,
        short: 'X',
        long: "exact",
        aliases: &["no-retry", "no-suggest", "strict"],
        arity: Arity::Switch,
        group: Group::Matching,
        summary: "Do not retry with a close spelling.",
        detail: "When a search finds nothing, syk looks through the same names and files for text that is one edit away from the pattern: one character missing, one added, one changed, or two neighbours swapped. In a terminal it prints a note and shows the results for the closest spelling, and the exit status is 0 if those exist.\n\nWhen the output is piped it only prints the suggestion and leaves the results empty, so a script never receives results for a different word than it asked for.\n\nOnly a single pattern of 4 to 40 letters, digits, dashes, underscores and spaces is corrected. Patterns with other characters, and searches that give -n, -d and -c their own patterns, are left alone.\n\nThe extra pass runs only after a search that found nothing. -X skips it.",
        examples: &[
            (
                "syk confg",
                "finds nothing, then shows the results for config",
            ),
            ("syk confg -X", "finds nothing and stops there"),
        ],
        related: &[Id::Literal, Id::Explain],
    },
    Flag {
        id: Id::Hidden,
        short: 'H',
        long: "hidden",
        aliases: &[],
        arity: Arity::Switch,
        group: Group::Walk,
        summary: "Include hidden files and folders.",
        detail: "Entries whose name starts with a dot are skipped by default. -H includes them, except for the inside of .git folders. Use -a to search those too. Hidden names in a group that is skipped, such as .venv and .next, stay out until -u or -a includes them.",
        examples: &[
            ("syk -H env", "also finds .env and .envrc"),
            ("syk -c token -H", "search dotfiles as well"),
        ],
        related: &[Id::NoIgnore, Id::All],
    },
    Flag {
        id: Id::NoIgnore,
        short: 'I',
        long: "no-ignore",
        aliases: &["ignored"],
        arity: Arity::Switch,
        group: Group::Walk,
        summary: "Include files that ignore rules exclude.",
        detail: "By default, anything matched by .gitignore, .ignore, .git/info/exclude or the global Git ignore file is skipped, and so are the groups of -g. That is what keeps node_modules, target and build output out of the results. -I turns those rules and every group off. Hidden entries stay skipped unless -H is also given.\n\nA search of / also skips /proc and /sys, which hold views of the kernel and no files. -I and -a search them, and so does naming one as the path.",
        examples: &[
            ("syk -I -d node_modules", "find the node_modules folders"),
            (
                "syk -I -n react -D 2",
                "names matching react, ignored ones included",
            ),
        ],
        related: &[Id::Hidden, Id::All, Id::Unignore],
    },
    Flag {
        id: Id::All,
        short: 'a',
        long: "all",
        aliases: &["unrestricted"],
        arity: Arity::Switch,
        group: Group::Walk,
        summary: "Include hidden and ignored entries.",
        detail: "The same as -H -I, and it also enters .git folders. Every group of -g is switched off as well. Binary files are still skipped when searching contents.",
        examples: &[(
            "syk -a secret",
            "search everything below the current folder",
        )],
        related: &[Id::Hidden, Id::NoIgnore],
    },
    Flag {
        id: Id::Ignore,
        short: 'g',
        long: "ignore",
        aliases: &["group", "groups", "skip-group"],
        arity: Arity::Required("GROUPS"),
        group: Group::Walk,
        summary: "Skip a named group of files, such as lock.",
        detail: "A group is a list of names that belong together, such as the folders that package managers fill. Give several separated by commas, or repeat the flag. deps and cache are skipped without asking, lock files are found by name but their contents are not searched, and the other groups are skipped when you name them. A flag in the config file makes it the default.\n\ndeps, skipped by default: node_modules, bower_components, jspm_packages, site-packages, venv, .venv, Pods, .pnpm-store.\n\ncache, skipped by default: __pycache__, *.pyc, *.pyo, .cache, .pytest_cache, .mypy_cache, .ruff_cache, .tox, .gradle, .turbo, .parcel-cache, .sass-cache, .eslintcache, .next, .nuxt, .svelte-kit, .DS_Store, Thumbs.db.\n\nbuild: target, dist, build, out, _build, coverage, *.o, *.class.\n\nlock, contents not searched by default: package-lock.json, yarn.lock, pnpm-lock.yaml, bun.lock, bun.lockb, Cargo.lock, composer.lock, Gemfile.lock, poetry.lock, uv.lock, Pipfile.lock, go.sum, flake.lock, mix.lock, pubspec.lock, Podfile.lock.\n\nmin: *.min.js, *.min.css, *.map.\n\nmedia: *.png, *.jpg, *.jpeg, *.gif, *.webp, *.avif, *.ico, *.bmp, *.svg, *.mp3, *.wav, *.ogg, *.flac, *.mp4, *.mov, *.mkv, *.webm, *.avi, *.woff, *.woff2, *.ttf, *.otf, *.eot.\n\nall stands for every group. A name is matched whole and at any depth, and *.ext matches the end of a name. A folder that matches is not entered, but a path you give yourself is always searched. -I and -a switch every group off.\n\nAdd :content to a name to keep finding the names and leave only the contents unsearched: -g min:content. That is how lock starts out, and -g lock hides the lock files as well. When lines in such a file match, a line on the terminal says how many are not shown.\n\nA value with an = sign makes a group of your own and skips it: -g 'snapshots=__snapshots__,*.snap'. With += the names are added to a group that exists: -g deps+=vendor. Any other pattern in a group of your own is read the way -E reads it, unless the group has :content.",
        examples: &[
            ("syk -n -g lock", "list names without the lock files"),
            (
                "syk -c function -g min:content",
                "do not read minified files",
            ),
            (
                "syk .js -g build,min",
                "skip build output and minified files",
            ),
            (
                "syk -c TODO -g deps+=vendor",
                "skip vendor together with deps",
            ),
            ("syk foo -g 'snaps=*.snap'", "a group of your own"),
        ],
        related: &[Id::Unignore, Id::Exclude, Id::NoIgnore],
    },
    Flag {
        id: Id::Unignore,
        short: 'u',
        long: "unignore",
        aliases: &["with", "keep", "search-group"],
        arity: Arity::Required("GROUPS"),
        group: Group::Walk,
        summary: "Search a group that is skipped by default.",
        detail: "Switches a group of -g off, so its files and folders are found again and their contents are searched. deps and cache are the two that are skipped without asking, and lock is the one whose contents are not searched. Give several separated by commas, or all for every group. Flags are read from left to right and the config file comes first, so -u on the command line also undoes a -g from the config file.\n\nIgnore rules still apply: a node_modules folder that .gitignore lists needs -I.",
        examples: &[
            (
                "syk -d node_modules -u deps",
                "find the node_modules folders",
            ),
            (
                "syk -c useState -u deps",
                "search installed packages as well",
            ),
            ("syk -c serde -u lock", "search inside lock files as well"),
            ("syk foo -u all", "skip no group"),
        ],
        related: &[Id::Ignore, Id::NoIgnore, Id::All],
    },
    Flag {
        id: Id::Follow,
        short: 'L',
        long: "follow",
        aliases: &["follow-links"],
        arity: Arity::Switch,
        group: Group::Walk,
        summary: "Follow symbolic links.",
        detail: "By default a symbolic link is matched by its own name and is not opened or entered. -L follows links to files and to folders. Loops are detected and skipped.",
        examples: &[("syk foo -L", "also search what links point to")],
        related: &[Id::Type],
    },
    Flag {
        id: Id::List,
        short: 'l',
        long: "list",
        aliases: &["files", "files-with-matches"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Print only the paths of files with a hit.",
        detail: "Each file is printed once, without its matching lines. Reading a file stops at its first match, so this does less work than a full search when you only need to know which files match.",
        examples: &[
            ("syk -c TODO -l", "which files contain TODO"),
            (
                "syk foo -l | xargs wc -l",
                "pass the paths on to another command",
            ),
        ],
        related: &[Id::Content, Id::Plain],
    },
    Flag {
        id: Id::Count,
        short: 'N',
        long: "count",
        aliases: &["counts"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Print the number of matching lines per file.",
        detail: "Prints each file once, followed by a colon and the number of lines that match, in place of the lines themselves. A file or folder that matches by name only is printed without a number.\n\nTogether with -T the totals are printed as well.",
        examples: &[
            ("syk -c TODO -N", "how many TODO lines each file has"),
            ("syk -c TODO -N -T", "the same, with the totals at the end"),
        ],
        related: &[Id::List, Id::Stats],
    },
    Flag {
        id: Id::Max,
        short: 'm',
        long: "max",
        aliases: &["max-count", "limit"],
        arity: Arity::Required("N"),
        group: Group::Output,
        summary: "Stop after N results.",
        detail: "A result is one printed path or one printed matching line. The whole search stops as soon as N of them are printed.\n\nWhich N you get is not fixed, because folders are searched in parallel. Add -o for a stable order.",
        examples: &[
            ("syk -n config -m 1", "is there one at all"),
            ("syk -c TODO -m 20 -o", "the first 20 in path order"),
        ],
        related: &[Id::Sort, Id::List],
    },
    Flag {
        id: Id::Context,
        short: 'C',
        long: "context",
        aliases: &[],
        arity: Arity::Required("N"),
        group: Group::Output,
        summary: "Show N lines before and after each matching line.",
        detail: "Context lines are printed with a dash after the line number where matching lines have a colon. Groups of lines that are not next to each other are separated by a line holding two dashes.",
        examples: &[("syk -c panic -C 3 .rs", "three lines around each panic")],
        related: &[Id::Content],
    },
    Flag {
        id: Id::Sort,
        short: 'o',
        long: "sort",
        aliases: &["ordered", "sorted"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Print results in path order.",
        detail: "Results normally appear in the order the worker threads find them, which differs from run to run. -o walks the tree on one thread in alphabetical order, so the output is the same every time. It takes longer on large trees.",
        examples: &[("syk foo -o", "the same output on every run")],
        related: &[Id::Threads, Id::Max],
    },
    Flag {
        id: Id::Rank,
        short: 'r',
        long: "rank",
        aliases: &["ranked", "relevance"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "Order results by relevance.",
        detail: "Collects every result before printing and orders them by a score. From heaviest to lightest, the score counts: how often you picked that path, or a folder above it, with -1; how well the name matches (the whole name, then the start of the name, then the start of a word in it, then anywhere); how often you visit the surrounding folder according to zoxide, when it is installed; how recently the file changed; and how shallow the path is.\n\nIn a terminal the best match is printed last, next to the prompt. When the output is piped the best match comes first.\n\nRanking is on by default in a terminal for searches that only look at names: -n, -d and their combinations. It is off by default when file contents are searched and when the output is piped, because nothing can be printed until the whole search is done. -r switches it on for those too. A content search is then ranked per file, not per line. With -m N the N best results are kept.\n\nA ranked name search also finds folders zoxide knows and paths you picked with -1 when they are hidden or ignored, as long as they still exist below the search path. You have been there, so they are not hidden from you.",
        examples: &[
            ("syk -n config", "ranked already, in a terminal"),
            ("syk -c TODO -r", "files ranked, the best one last"),
            ("syk -n config -r | head -5", "the five best, best first"),
        ],
        related: &[Id::NoRank, Id::Best, Id::Why, Id::Sort],
    },
    Flag {
        id: Id::NoRank,
        short: 'R',
        long: "no-rank",
        aliases: &["unranked", "stream"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "Print results as they are found.",
        detail: "Switches ranking off for a name search in a terminal. Results then appear immediately, in the order the worker threads find them. -o, which sorts by path, switches ranking off as well.",
        examples: &[("syk -n config -R", "unranked, printed while searching")],
        related: &[Id::Rank, Id::Sort],
    },
    Flag {
        id: Id::Best,
        short: '1',
        long: "best",
        aliases: &["first", "top", "pick"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "Print only the best match, and remember it.",
        detail: "Ranks the results as -r does and prints the single best path, without matching lines. It is meant for handing a path to another command.\n\nEach path printed this way is remembered in a small file: ~/.local/share/syk/picks, or below $XDG_DATA_HOME when that is set. A remembered path, and everything below a remembered folder, ranks higher in later searches. The scheme is the one zoxide uses: every pick adds one point, recent picks count for more, and old entries fade out.\n\nNothing else is recorded: not your patterns, and not results you only looked at. Delete the file to forget everything.",
        examples: &[
            (
                "cd (syk -d dora -1)",
                "jump to the best matching folder, in fish",
            ),
            ("nvim $(syk -n config -1)", "open the best matching file"),
            ("syk -c TODO -1", "the best file that contains TODO"),
        ],
        related: &[Id::Rank, Id::Why, Id::Picks, Id::Max],
    },
    Flag {
        id: Id::Browse,
        short: 'b',
        long: "browse",
        aliases: &["choose", "select"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "Choose one result from a list, and remember it.",
        detail: "Ranks the results as -r does and shows them in a list instead of printing them, best first. j, k and the arrow keys move, n and b move a screen at a time, / narrows the list as you type, Enter chooses and q cancels.\n\nThe chosen path is printed, so it can be handed to another command, and it is remembered like a pick made with -1. A matching line is a result of its own: with -O the editor opens at that line. With -f the command runs on the chosen path only.\n\nThe list is drawn on the terminal itself, so it also works inside a command substitution. Cancelling prints nothing and exits with status 1.",
        examples: &[
            ("syk -n config -b", "choose a file and print its path"),
            (
                "syk -c TODO -b -O",
                "choose a TODO line and open the file there",
            ),
            (
                "cd (syk -d api -b)",
                "choose a folder and go there, in fish",
            ),
        ],
        related: &[Id::Best, Id::Open, Id::Exec, Id::Picks],
    },
    Flag {
        id: Id::Why,
        short: 'W',
        long: "why",
        aliases: &["score", "scores"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "Show each result's score and what it is made of.",
        detail: "Ranks the results as -r does and prints one line per result: the total score, the five parts it is the sum of, and the path. Matching lines are left out.\n\npicked, at most 4: how often -1 chose this path or a folder above it, with recent picks counting for more. name, at most 3: how well the name matches, from the whole name down to somewhere in the middle of it. visited, at most 2: zoxide's score for this folder or one above it. recent, at most 1: how recently the file changed, in steps for an hour, a day, a week and a month. shallow, at most 0.5: fewer folders deep is better.\n\nFor picked and visited, a folder further above the path counts for less than the path itself, and the value is relative to your most used entry.\n\nTogether with -1 it shows what -1 would choose, and nothing is remembered.",
        examples: &[
            ("syk -d dotfiles -W", "why the folders rank the way they do"),
            ("syk -n config -W -m 5", "the five best, with their scores"),
            (
                "syk -d dotfiles -1 -W",
                "what -1 would pick, without remembering it",
            ),
        ],
        related: &[Id::Rank, Id::Best, Id::Picks],
    },
    Flag {
        id: Id::Picks,
        short: 'k',
        long: "picks",
        aliases: &["history", "remembered"],
        arity: Arity::Switch,
        group: Group::Ranking,
        summary: "List the paths remembered by -1, -b and -O.",
        detail: "Prints every remembered path with its current score, how often it was picked and when it was last picked, best first. The score is the number of picks, multiplied by 4 within the last hour, by 2 within the last day, by 0.5 within the last week and by 0.25 after that.\n\nIt also reports where the file is kept and how many folders zoxide knows. No search is run.",
        examples: &[("syk -k", "what syk has learned from your picks")],
        related: &[Id::Best, Id::Why],
    },
    Flag {
        id: Id::Open,
        short: 'O',
        long: "open",
        aliases: &["edit"],
        arity: Arity::Switch,
        group: Group::Action,
        summary: "Open the best match in your editor.",
        detail: "Ranks the results as -1 does, takes the best one and starts the program named in $VISUAL or $EDITOR on it. When the match is a line inside a file, the editor opens at that line. That works for vi, vim, nvim, nano, emacs, micro, kak, helix, kate, VS Code and its relatives, Zed and Sublime Text. Any other editor gets the path alone. Without $VISUAL and $EDITOR the path is opened with xdg-open, or with open on macOS.\n\nThe opened path is remembered the way -1 remembers it, so it ranks higher next time.",
        examples: &[
            ("syk -c 'fn main' -O", "open the best file at fn main"),
            ("syk -n config -O", "open the best matching file"),
        ],
        related: &[Id::Best, Id::Exec, Id::Why],
    },
    Flag {
        id: Id::Exec,
        short: 'f',
        long: "exec",
        aliases: &["for-each", "each", "run", "do"],
        arity: Arity::Required("COMMAND"),
        group: Group::Action,
        summary: "Run a command on the results.",
        detail: "Runs COMMAND through sh with the result paths as its arguments, in place of printing them. Without {} in it, the command runs once with every path added at the end, split over several runs when there are very many. With {} in it, the command runs once per path, with {} standing for that path. Write {} without quotes around it: paths with spaces or other special characters are passed on safely either way.\n\nPaths are handed over in path order, or best first with -r. -m limits how many there are, and -1 passes only the best one. Quote the command so that it stays one argument.\n\nThe exit status is 1 when a command fails.",
        examples: &[
            (
                "syk -c TODO -f 'wc -l'",
                "count the lines of every file with a TODO",
            ),
            ("syk .log +30d -f 'gzip {}'", "compress each old log"),
            ("syk .orig -f 'rm -i'", "delete, asking first"),
            ("syk .png +1mb -f 'mv {} big/'", "move each large image"),
        ],
        related: &[Id::Null, Id::Open, Id::List, Id::Max],
    },
    Flag {
        id: Id::Plain,
        short: 'p',
        long: "plain",
        aliases: &["no-color", "no-colour"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "One result per line, without colour or grouping.",
        detail: "This is the default when the output goes to a pipe or a file. Folder hits are printed as path/ and file name hits as path. Content hits are printed as path:line:text. -p forces this format in a terminal.",
        examples: &[("syk foo -p", "the format scripts receive")],
        related: &[Id::Pretty, Id::List],
    },
    Flag {
        id: Id::Pretty,
        short: 'P',
        long: "pretty",
        aliases: &["color", "colour"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Colour and grouped results, even into a pipe.",
        detail: "This is the default in a terminal. Each file is printed once as a heading with its matching lines below it, and the matched text is highlighted. Very long lines are cut down to the part around the first match. -P forces this format when the output is piped.\n\nSetting the NO_COLOR environment variable keeps the grouping and drops the colour.",
        examples: &[("syk foo -P | less -R", "page through coloured results")],
        related: &[Id::Plain],
    },
    Flag {
        id: Id::Json,
        short: 'J',
        long: "json",
        aliases: &["ndjson", "jsonl"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Print one JSON object per result.",
        detail: "Each line of output is one JSON object, the format known as JSON Lines. A name hit is {\"type\":\"path\",\"path\":\"src/api\",\"kind\":\"dir\"}, with kind file or dir. A matching line is {\"type\":\"match\",\"path\":\"a.ts\",\"line\":2,\"text\":\"...\",\"spans\":[[6,12]]}, where spans holds the byte offsets of every match inside text. Lines printed by -C have the type context, and -N prints {\"type\":\"count\",\"path\":\"a.ts\",\"count\":3}.\n\nBytes that are not valid UTF-8 are replaced. A path with such bytes also gets path_hex, which holds its exact bytes. Colour is off, and long lines are not shortened.",
        examples: &[
            ("syk -c TODO -J", "every TODO line as a JSON object"),
            ("syk -c TODO -J | jq -r .path", "read it with jq"),
        ],
        related: &[Id::Plain, Id::Null, Id::List],
    },
    Flag {
        id: Id::Null,
        short: '0',
        long: "null",
        aliases: &["print0"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Print paths separated by NUL bytes.",
        detail: "Prints only paths, as -l does, and ends each one with a NUL byte in place of a newline. Names with spaces or newlines in them then survive a pipe into xargs -0 or a similar tool.",
        examples: &[(
            "syk .log +30d -0 | xargs -0 rm",
            "delete old logs, whatever their names",
        )],
        related: &[Id::List, Id::Exec, Id::Json],
    },
    Flag {
        id: Id::NoLinks,
        short: 'K',
        long: "no-links",
        aliases: &["no-link", "no-hyperlinks"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "Do not make paths clickable.",
        detail: "In a terminal, each path in the coloured output is a hyperlink to the file, which many terminals open on a click or a ctrl+click. A terminal without support for links shows the path as plain text. This flag switches the links off, for a terminal that prints them wrongly.\n\nLinks are never written into a pipe, into plain output, without colour, or in an SSH session.",
        examples: &[("syk foo -K", "paths as plain text")],
        related: &[Id::Plain, Id::Pretty],
    },
    Flag {
        id: Id::Stats,
        short: 'T',
        long: "stats",
        aliases: &["summary", "total", "totals"],
        arity: Arity::Switch,
        group: Group::Output,
        summary: "End with a line of totals and the time taken.",
        detail: "After the results, prints one line on standard error: how many lines, files and names matched, how many files were read out of how many entries were visited, and how long the search took. With an index it also says that the names came from it, and how many files were not read because the index showed that they could not match. Because it goes to standard error it does not end up in a pipe or in a file.\n\nWith -l, -1 and other output that holds paths only, a file is read up to its first matching line, so matching lines are not counted.",
        examples: &[
            ("syk -c TODO -T", "the results, then the totals"),
            ("syk -c TODO -N -T", "counts per file, then the totals"),
        ],
        related: &[Id::Count, Id::Explain],
    },
    Flag {
        id: Id::Explain,
        short: 'x',
        long: "explain",
        aliases: &["dry-run"],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Show how the arguments are read. Does not search.",
        detail: "Prints each pattern and how it will be read, what it is matched against, the search paths and the active filters. Use it when a search finds something unexpected, or nothing at all.",
        examples: &[("syk foo src .ts -x", "check which word became the pattern")],
        related: &[Id::Help],
    },
    Flag {
        id: Id::Threads,
        short: 'j',
        long: "threads",
        aliases: &["jobs"],
        arity: Arity::Required("N"),
        group: Group::Other,
        summary: "Use N worker threads.",
        detail: "The default is the number of CPU cores, up to twelve. A search starts on one thread. The others join after 128 entries when contents are searched and after 1,024 when only names are, or when the search takes longer than two milliseconds. -j 1 searches on a single thread.",
        examples: &[("syk foo -j 4", "use four threads")],
        related: &[Id::Sort],
    },
    Flag {
        id: Id::Index,
        short: 'y',
        long: "index",
        aliases: &["reindex"],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Build the index of a large folder now.",
        detail: "An index is a list of every name below a folder, kept in ~/.cache/syk. A search that finds one checks each folder in it for changes, which takes one look per folder, and reads only the folders that changed. That is far less work than walking a very large tree, and it finds the same as a search without an index.\n\nThe index also keeps a signature of the text in every file: which runs of three characters it holds, in any case. A search of contents works out which runs its pattern needs, and does not read a file that lacks one, after checking that the size and the times of the file are still the ones in the index. A file that changed is read as usual, and so is every file when the pattern has no three fixed characters in a row. When many files changed, syk builds the index again in the background. A signature takes about a twentieth of the size of the text. Files over 64 MiB get none, one index holds at most 256 MiB of them, and all indexes together about 1 GiB, where the ones built last are kept.\n\nsyk builds an index by itself, in the background, after a search that walked more than 20,000 entries. -y builds one right away and waits for it. Give the folder as a path: syk -y ~/.\n\nAn index holds what one kind of search sees. A plain search and a search with -H each get their own. One for -I or -a is used when it exists, and is only built when you ask for it: syk -y -a. A search with -L or in several paths uses none.\n\nA search of a folder inside an indexed folder uses that index, so one index of your home folder serves every folder below it. A disk mounted inside the folder is part of the index when it is a local Linux file system, and is read on every search when it is not. It works on local Linux file systems only. The 24 indexes built last are kept, readable by you alone. -T says whether the names of a search came from an index, and how many files it showed could not match.",
        examples: &[
            ("syk -y", "index the current folder"),
            ("syk -y ~/", "index your home folder"),
            ("syk -y -H ~/", "the same for searches with -H"),
        ],
        related: &[Id::NoIndex, Id::Indexes, Id::Stats],
    },
    Flag {
        id: Id::NoIndex,
        short: 'Y',
        long: "no-index",
        aliases: &["live"],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Search without the index.",
        detail: "Walks the folders as they are now and neither reads nor builds an index. The results are the same either way, so this is for measuring, or for a file system where the times of folders cannot be trusted. Put it in the config file to switch indexes off for good.",
        examples: &[("syk -n config -Y ~/", "walk the whole tree")],
        related: &[Id::Index, Id::Indexes],
    },
    Flag {
        id: Id::Indexes,
        short: 'A',
        long: "indexes",
        aliases: &["indices"],
        arity: Arity::Optional("ACTION"),
        group: Group::Other,
        summary: "List, clear or prune the stored indexes.",
        detail: "Without ACTION, or with list, it shows every index with its size, the number of entries, when it was built and its folder. Give a folder as a path to see only that one.\n\nclear removes every index, or only the ones of the folders given as paths. prune removes the indexes whose folder no longer exists and the ones this version cannot read.\n\nRemoving an index changes no results. The next search of a large folder walks it and builds a new index in the background, unless -Y is in the config file. A build that is running finishes and leaves its index.",
        examples: &[
            ("syk -A", "every index and its size"),
            ("syk -A ~/code", "the index of one folder"),
            ("syk -A clear", "remove every index"),
            ("syk -A clear ~/code", "remove the index of one folder"),
            ("syk -A prune", "remove indexes of folders that are gone"),
        ],
        related: &[Id::Index, Id::NoIndex],
    },
    Flag {
        id: Id::NoPager,
        short: 'G',
        long: "no-pager",
        aliases: &[],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Print help and other views in one piece.",
        detail: "Help, the welcome screen and the output of -x and -k open in a pager when they are taller than the window. -G prints them in one piece instead. Search results are never paged.\n\nPut -G in the config file to switch the pager off for good. Setting SYK_PAGER=off does the same.\n\nBecause -G changes how help is shown, it does not ask for its own page when it stands next to -h. syk -h no-pager shows this page.",
        examples: &[
            ("syk -h -G", "the overview in one piece"),
            ("syk -h no-pager", "this page"),
        ],
        related: &[Id::Help, Id::NoBanner, Id::NoConfig],
    },
    Flag {
        id: Id::NoBanner,
        short: 'B',
        long: "no-banner",
        aliases: &["no-logo"],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Leave the logo out of help and other views.",
        detail: "In a terminal, help, the welcome screen, -x, -k and --setup start with the logo. -B leaves it out. Help and the welcome screen then start with the two-line title that piped output gets.\n\nPut -B in the config file to leave the logo out for good.\n\nLike -G, it does not ask for its own page when it stands next to -h. syk -h no-banner shows this page.",
        examples: &[
            ("syk -h -B", "the overview without the logo"),
            ("syk foo -x -B", "how foo is read, without the logo"),
            ("syk -h no-banner", "this page"),
        ],
        related: &[Id::Help, Id::NoPager, Id::NoConfig],
    },
    Flag {
        id: Id::NoConfig,
        short: 'U',
        long: "no-config",
        aliases: &["no-defaults"],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Ignore the config file.",
        detail: "syk reads default flags from a config file: the file named in $SYK_CONFIG, or else syk/config below $XDG_CONFIG_HOME, or else ~/.config/syk/config. The file holds flags and their values exactly as you would type them, on one line or on several. A # starts a comment. Patterns, paths and bare filters such as .ts do not belong in it.\n\nThe flags from the file come before the ones you type, so where two flags disagree, such as -p and -P, the typed one wins. A switch that the file turns on cannot be turned off again from the command line. -U skips the file for one run, and -x shows which defaults are active.",
        examples: &[
            ("syk foo -U", "search without your defaults"),
            ("syk foo -x", "see which defaults apply"),
        ],
        related: &[Id::Explain],
    },
    Flag {
        id: Id::Init,
        short: 'z',
        long: "init",
        aliases: &["shell"],
        arity: Arity::Required("SHELL"),
        group: Group::Other,
        summary: "Print shell setup: completions and a jump command.",
        detail: "Prints a script for fish, zsh or bash that adds tab completion for every flag, and a function called s that changes to the best matching folder: s dora. It looks below the current folder first and below your home folder after that, and every jump is remembered the way -1 remembers it. s without arguments goes home.\n\nLoad it from the startup file of your shell. In fish: syk --init fish | source. In zsh: eval \"$(syk --init zsh)\". In bash: eval \"$(syk --init bash)\".\n\nA name after a colon gives the function another name: syk --init fish:j.\n\nsyk --setup adds that line for you, in the place where your shell configuration loads its other tools.",
        examples: &[
            ("syk --init fish | source", "set up fish"),
            (
                "syk --init zsh:j",
                "the zsh script, with the jump command called j",
            ),
        ],
        related: &[Id::Setup, Id::Best, Id::Picks],
    },
    Flag {
        id: Id::Setup,
        short: 'Z',
        long: "setup",
        aliases: &["install"],
        arity: Arity::Optional("SHELL"),
        group: Group::Other,
        summary: "Add the shell setup to your shell configuration.",
        detail: "Finds where your shell configuration lives and adds the line that loads the output of --init, so that completions and the jump command s work in every new shell. SHELL is fish, zsh or bash, and defaults to the shell in $SHELL. A name after a colon renames the jump command: syk --setup fish:j.\n\nIt does not assume a standard layout. It follows symbolic links and sourced files from the file your shell reads first, and looks at how tools such as zoxide and starship are loaded there. When such a tool has a small file of its own, syk gets one next to it, written the same way. When it is a line in a larger file, syk gets a line next to it. Otherwise the standard place is used: conf.d for fish, and .zshrc or .bashrc for the others.\n\nNothing is written before you have seen the change and answered y. After writing, a new shell is started to check that s exists. When it does not, the change is undone and the standard place is offered.\n\nsyk --setup agent prints instructions for an AI coding agent, for a configuration that someone has to read first.",
        examples: &[
            ("syk --setup", "set up the shell you are using"),
            (
                "syk --setup zsh:j",
                "set up zsh, with the jump command called j",
            ),
            ("syk --setup agent", "instructions for a coding agent"),
        ],
        related: &[Id::Init, Id::NoConfig],
    },
    Flag {
        id: Id::Help,
        short: 'h',
        long: "help",
        aliases: &[],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Show help, or the page of the flag next to it.",
        detail: "On its own, -h prints the overview of every flag. Together with other flags it prints the full page of each of those flags instead. The long name of a flag works as a bare word too. Given twice, -h prints this page. A word that is not the name of a flag is looked up in every flag page: syk -h pipe lists the flags whose page mentions pipe.\n\nIn a terminal, a page that is taller than the window opens in a pager. n or Space shows the next screen, which starts at a section heading, and b goes back. j, k and the arrow keys move one line, g and G go to the start and the end. q closes the pager and leaves the screen you were reading in view. The bottom line names the section each key leads to. s lists the sections so you can jump to one, - followed by the letter of a flag opens the page of that flag, and ? lists every key.\n\n/ searches the page. The page follows what you type, lower case matches any case, and one upper case letter makes the case exact. Enter keeps the search: n and b then step through the matching lines, and Esc goes back to paging.\n\nUse -G, or set SYK_PAGER=off, to print a page in one piece. Output that goes into a pipe or a file is never paged.",
        examples: &[
            ("syk -h", "the overview"),
            ("syk -n -h", "the page for -n, --name"),
            ("syk --modified --help", "the page for -M, --modified"),
            ("syk -h size", "the page for -S, --size"),
            ("syk -h -h", "this page"),
            ("syk -h pipe", "the flags whose page mentions pipe"),
            ("SYK_PAGER=off syk -h", "the overview, without paging"),
        ],
        related: &[Id::Explain],
    },
    Flag {
        id: Id::Version,
        short: 'V',
        long: "version",
        aliases: &[],
        arity: Arity::Switch,
        group: Group::Other,
        summary: "Print the version.",
        detail: "Prints the name and version number and exits.",
        examples: &[("syk -V", "print the version")],
        related: &[Id::Help],
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_used_once() {
        for (index, flag) in FLAGS.iter().enumerate() {
            let names = std::iter::once(flag.long).chain(flag.aliases.iter().copied());
            for other in &FLAGS[index + 1..] {
                assert_ne!(flag.short, other.short, "-{} is used twice", flag.short);
                for name in names.clone() {
                    assert!(
                        other.long != name && !other.aliases.contains(&name),
                        "--{name} is used twice"
                    );
                }
            }
            assert_eq!(by_id(flag.id).long, flag.long);
        }
    }
}
