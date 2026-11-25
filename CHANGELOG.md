# Changelog

## 0.1.4

This release changes how syk looks in the terminal. Searching works the same as in 0.1.3.

- The banner above the help and `--setup` shows the new logo. Matches, folders and line numbers use colours that go with it.
- `syk --setup welcome` shows a welcome card with a few commands to try first. `--setup` shows the same card after it writes the shell setup.
- The install script ends with a hint to run `syk --setup`, and the AUR package shows the welcome card after it installs.

## 0.1.3

This release is about using syk in more places. It can search what is piped in, the index now works on macOS, and two new filters narrow a search to the files you are working on or to one language. Building an index again now lists only the folders that changed.

Installing is also more reliable. The Linux builds need glibc 2.34, but the install script accepted 2.31, so on Debian 11 and similar systems it installed a program that could not start. Those systems now get a static build. From this version on, a tagged release publishes itself to crates.io and the AUR.

### Searching

- `syk pattern -` searches what is piped in and prints each match as it arrives. `syk -- -` still searches for a dash.
- `-Q`, `--uncommitted` searches only the files that git reports as changed or new.
- `-e` takes language names such as `rust`, `typescript` and `web`, which stand for all their extensions. A plain extension stays exact.
- A name search for a plain word no longer goes through the regular expression engine, and every thread keeps its own patterns, which makes searches of names without an index faster.

### Index

- The index works on macOS, on APFS and HFS+.
- Building an index again copies the folders that did not change from the old index and lists only the ones that did.

### Installing

- Releases include static Linux builds, which the install script uses on systems with a glibc older than 2.34.
- `cargo binstall syk-search` downloads the built program, and every release archive has a GitHub attestation.
- `syk --setup man` installs the manual page next to the program, for installs through cargo or the install script.
- `syk --init man` prints the manual page, and `syk --init zsh:` prints the completions without the jump command, for packages.
- The AUR package installs the manual page and the completions.

### Benchmarks

The [benchmark page](https://remcostoeten.github.io/syk/benchmarks.html) compares syk with ripgrep, ugrep, git grep, fd, bfs, find, plocate and grep, on searches where every tool prints the same results. A regular expression in a tree of about 81,000 files took a median of 121 ms with syk, 45 ms with its index and 143 ms with ripgrep, on a 16-thread machine that was also doing other work.

plocate is faster on names: 2 ms against 57 ms for syk without an index on a rare name in `/usr`. plocate does not see changes made after its last index build, while syk checks for them on every search.

## 0.1.2

On Linux, syk now keeps an index. It remembers the names below a folder and a signature of the text of every file. A later search uses that after checking every folder for changes, so it finds the same as a search that reads everything. There is nothing to set up: the first search from a terminal below your home folder builds an index of the whole home folder in the background, and every folder below it uses that one index. `syk -A` lists what is stored and removes it again.

The other part of this release is what a search leaves out. Dependency folders and caches are skipped as named groups, `-u` searches a group again and `-x` lists them. A search that finds nothing says when a skipped place does have results, and which flag shows them.

Three changes can affect scripts: the exit status is 2 when a file or folder could not be read, an empty word is an error, and a search of `/` skips `/proc` and `/sys`.

### Index

- syk keeps an index of the names below a folder in `~/.cache/syk`. A search checks every folder in it for changes and reads only the ones that changed, so it finds the same as a search without an index. Linux only.
- The index also holds a signature of the text of every file. A search of contents does not read the files that cannot hold its pattern, after checking that they did not change since the index was built. `-T` says how many files that were.
- The first search from a terminal below the home folder builds an index of the whole home folder in the background, and says so once. Outside it, a search that walks more than 20,000 entries builds one of the folder it searched.
- A search in a folder below an indexed folder uses that index, so one index of the home folder serves every folder below it. An index of a folder replaces the indexes of the folders inside it.
- A search with `-H` gets an index of its own, built in the background like the plain one. `syk -y -I` and `syk -y -a` build one for those searches.
- A local Linux file system that is mounted inside an indexed folder is part of the index. Any other mount is read on every search.
- Background builds run one at a time at the lowest CPU and disk priority. None starts while less than 2 GB is free, and the first one of the home folder waits on a computer that runs on its battery.
- A new index takes over the signatures of files that did not change, so building one again reads only the files that did.
- `-y` builds an index right away, `-Y` searches without one, and `-T` says whether the names came from an index.
- `-A` lists the stored indexes with their size, age and folder. `-A clear` removes all of them or those of the given folders, and `-A prune` removes those whose folder is gone. The 24 indexes used last are kept.

### Searching

- On Linux, syk lists folders with a walker of its own and applies the ignore rules with the same matchers as before. `-L` and `-o` still use the walker of the `ignore` crate.
- Groups of names are skipped as a whole. `deps` (node_modules, venv and other dependency folders) and `cache` (__pycache__, .next and other caches) are skipped without asking, also where no ignore file lists them.
- Lock files (`lock`) are found by name, but their contents are not searched. When lines in a lock file match, a line on the terminal says how many are not shown.
- `-g` skips a group: `build`, `lock`, `min` and `media` exist next to the two above, `-g min:content` leaves only the contents unsearched, and `-g 'name=a,b'` makes a group of your own.
- `-u` searches a group again, and `-I` and `-a` switch every group off.
- `-x` lists the groups that are skipped and the names in them.
- A search of `/` skips `/proc` and `/sys`, unless `-I` or `-a` is given or one of them is named as the path.
- A search that finds nothing says on the terminal when hidden entries, ignored files or a skipped group do have results, and which flag shows them. `-X` skips that look.

### Changed

- The exit status is 2 when a file or folder could not be read, also when other results were found.
- An empty word, as an empty shell variable gives, is an error and no longer prints every name and line.
- `-J` adds `path_hex` to a path that is not valid UTF-8, so the exact file can still be found.
- An error of a walk names its path once.

Full diff: [v0.1.1...v0.1.2](https://github.com/remcostoeten/syk/compare/v0.1.1...v0.1.2), 42 commits in 56 files.

## 0.1.1

- A file under 64 KiB is read in one call, and reading stops at the first zero byte of a binary file.
- A query with several words scans for the longest word and checks the others on that line.
- On Linux, a file whose data is not in memory gets a read-ahead request and is searched a moment later, so the disk works on several files at once.
- A tree of up to 128 entries starts on one thread, and the others join only when the search takes longer than two milliseconds.
- The default number of worker threads is the number of CPU cores, up to twelve.

## 0.1.0

First release.
