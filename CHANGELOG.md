# Changelog

## Unreleased

- After a search that walks more than 20,000 entries, syk builds an index of the names in the background and keeps it in `~/.cache/syk`. Later searches below that folder check every folder in the index for changes and read only the ones that changed, so they find the same as a search without an index. Linux only.
- The index also holds a signature of the text of every file. A search of contents does not read the files that cannot hold its pattern, after checking that they did not change since the index was built. `-T` says how many files that were.
- On Linux, syk lists folders with a walker of its own and applies the ignore rules with the same matchers as before. `-L` and `-o` still use the walker of the `ignore` crate.
- `-y` builds an index right away, `-Y` searches without one, and `-T` says whether the names came from an index.
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
