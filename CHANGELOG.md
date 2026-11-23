# Changelog

## Unreleased

- The first search from a terminal below the home folder builds an index of the whole home folder in the background, and says so once. Outside it, a search that walks more than 20,000 entries builds one of the folder it searched. The indexes are kept in `~/.cache/syk`.
- Background builds run one at a time at the lowest CPU and disk priority, and none starts while less than 2 GB is free.
- An index of a folder replaces the indexes of the folders inside it, and the 24 indexes used last are kept.
- An index holds the names below a folder. Later searches below that folder check every folder in the index for changes and read only the ones that changed, so they find the same as a search without an index. Linux only.
- The index also holds a signature of the text of every file. A search of contents does not read the files that cannot hold its pattern, after checking that they did not change since the index was built. `-T` says how many files that were.
- On Linux, syk lists folders with a walker of its own and applies the ignore rules with the same matchers as before. `-L` and `-o` still use the walker of the `ignore` crate.
- `-y` builds an index right away, `-Y` searches without one, and `-T` says whether the names came from an index.
- A search in a folder below an indexed folder uses that index, so one index of the home folder serves every folder below it.
- A search with `-H` gets an index of its own, built in the background like the plain one. `syk -y -I` and `syk -y -a` build one for those searches.
- A disk mounted inside an indexed folder no longer stops the build. A local Linux file system becomes part of the index, and any other mount is read on every search.
- A new index takes over the signatures of files that did not change, so building one again reads only the files that did.
- A search of `/` skips `/proc` and `/sys`, unless `-I` or `-a` is given or one of them is named as the path.
- `-A` lists the stored indexes with their size, age and folder. `-A clear` removes all of them or those of the given folders, and `-A prune` removes those whose folder is gone.
- The exit status is 2 when a file or folder could not be read, also when other results were found.
- `-J` adds `path_hex` to a path that is not valid UTF-8, so the exact file can still be found.
- Groups of names are skipped as a whole. `deps` (node_modules, venv and other dependency folders) and `cache` (__pycache__, .next and other caches) are skipped without asking, also where no ignore file lists them.
- Lock files (`lock`) are found by name, but their contents are not searched.
- `-g` skips a group: `build`, `lock`, `min` and `media` exist next to the two above, `-g min:content` leaves only the contents unsearched, and `-g 'name=a,b'` makes a group of your own.
- `-u` searches a group again, and `-I` and `-a` switch every group off.
- `-x` lists the groups that are skipped and the names in them.
- When lines in a lock file match, a line on the terminal says how many are not shown.

## 0.1.1

- A file under 64 KiB is read in one call, and reading stops at the first zero byte of a binary file.
- A query with several words scans for the longest word and checks the others on that line.
- On Linux, a file whose data is not in memory gets a read-ahead request and is searched a moment later, so the disk works on several files at once.
- A tree of up to 128 entries starts on one thread, and the others join only when the search takes longer than two milliseconds.
- The default number of worker threads is the number of CPU cores, up to twelve.

## 0.1.0

First release.
