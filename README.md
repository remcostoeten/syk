<h1 align="center">syk</h1>

**Syk** *(verb)*
/sik/, *Frisian, from "sykje", to search.*

A command line tool that searches folder names, file names and file contents in one pass. You give it a pattern, a path and filters in any order, and it works out which is which.

<p align="center">
  <img src="https://shieldcn.dev/github/remcostoeten/syk/release.svg?font=jetbrains-mono" alt="release" />
  <img src="https://shieldcn.dev/github/remcostoeten/syk/ci.svg?font=jetbrains-mono" alt="CI" />
  <img src="https://shieldcn.dev/github/remcostoeten/syk/license.svg?font=jetbrains-mono" alt="license" />
  <img src="https://shieldcn.dev/badge/core-Rust-black.svg?font=jetbrains-mono&logo=rust" alt="core: Rust" />
  <img src="https://shieldcn.dev/badge/search-ripgrep%20crates-black.svg?font=jetbrains-mono" alt="search: ripgrep crates" />
  <img src="https://shieldcn.dev/badge/runs%20on-Linux%20and%20macOS-black.svg?font=jetbrains-mono" alt="runs on: Linux and macOS" />
</p>

<p align="center">
  <img src="docs/demo.gif" width="100%" alt="syk searches for login below src, narrows a content search to files named route, lists old log files, lets you choose one result from a list, and pages through its help with a search" />
</p>
<p align="center">
  <sub>Recorded with VHS in bash.</sub>
</p>

`syk foo` looks for foo in folder names, in file names and in the lines of files, and prints every hit. It walks and searches with the crates behind ripgrep, so hidden, ignored and binary files are skipped unless you ask for them.

- **Arguments in any order.** `syk foo src/ .ts +10kb -2h` is a pattern, a path, an extension, a minimum size and a maximum age, recognised by their shape.
- **One target or several.** `-n`, `-d` and `-c` search names, folders or contents only, and `-n route -c auth` prints the lines matching auth in files named route.
- **Groups of names to skip.** Dependency folders and caches such as `node_modules` and `__pycache__` are skipped without an ignore file, lock files are found by name while their contents stay out of the results, and `-u deps` searches a group again.
- **An index for very large folders.** After a search that walks more than 20,000 entries, syk keeps a list of the names and a signature of the text of every file in `~/.cache/syk`. Later searches check each folder in it for changes and read only the ones that changed, and a search of contents reads only the files that can hold its pattern. Linux only.
- **Forgiving patterns.** Lower case matches any case, a pattern that is not a valid regular expression is searched as plain text, and a spelling one edit away is tried when nothing is found.
- **Ranked results.** A name search in a terminal is ordered by how well the name matches, how recent the file is and what you picked before, with the best result next to your prompt.
- **Act on a result.** `-b` lets you choose one from a list, `-1` prints the best path, `-O` opens it in your editor at the matching line and `-f` runs a command on the results.
- **Output for scripts.** Piped output is plain `path:line:text`, `-J` prints JSON and `-0` separates paths with NUL bytes.
- **Help you can page through.** `syk -h` opens in a pager with section jumps and search, every flag has its own page, and `syk foo -x` shows how the arguments are read without searching.

`scripts/demo.sh` walks through every flag in a throwaway project.

## Install

### Prebuilt binary

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/remcostoeten/syk/releases/latest/download/syk-search-installer.sh | sh
```

### From source

```bash
cargo install --git https://github.com/remcostoeten/syk
```

`syk --setup` then adds tab completion and the `s` jump command to your shell.

## Development

Rust 1.88 or newer.

```bash
cargo test                                     # unit tests and end-to-end tests
cargo clippy --all-targets                     # lints
scripts/demo.sh --fast                         # run every documented command once
scripts/bench.py --html docs/benchmarks.html   # time searches next to ripgrep, fd, find and grep
scripts/fuzz.py                                # change random trees and compare the index with a walk
git config core.hooksPath .githooks            # format on commit, lint on push
```

<br/>

xxx,<br/>
[Remco Stoeten](https://remcostoeten.com)<br/>
<small>MIT</small>
