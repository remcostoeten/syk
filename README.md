# syk

**syk**  
*noun / verb* · /sɪk/  

from West Frisian *sykje* /ˈsikjə/ “to search, to seek”  

1. a fast Rust CLI that searches folder names, file names and file contents in one pass  
2. the act of finding things extremely quickly  

[![Crates.io](https://img.shields.io/crates/v/syk-search.svg)](https://crates.io/crates/syk-search)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Platform](https://img.shields.io/badge/platform-Linux%20%7C%20macOS-lightgrey)]()

```bash
syk foo src/ .ts +10kb -2h
```

Arguments in any order. Recognised by shape.  
One tool for names, folders, and contents.

---

### Why syk?

Most search tools do one job well.  
**syk** does three — and does them together.

| You want…                    | Traditional way              | syk way                  |
|-----------------------------|------------------------------|--------------------------|
| Find files by name          | `fd` / `find`                | `syk -n`                 |
| Find folders                | `fd -t d`                    | `syk -d`                 |
| Search contents             | `rg` / `grep`                | `syk -c`                 |
| All of the above at once    | Pipe tools together          | Just `syk`               |

---

### Features at a glance

- **Smart argument parsing** — order doesn’t matter  
  `syk auth src/ .rs -2h` just works
- **Combined filters** — they narrow each other  
  `syk -d api -c auth` → only search for “auth” under folders named “api”
- **Sensible defaults** — skips hidden, ignored, binary, dependency folders, caches and lock-file contents  
  (use `-u` to bring any group back)
- **Background index** — lives in `~/.cache/syk`  
  Only re-reads folders that actually changed
- **Smart ranking** — name results ordered by match quality, recency and past picks
- **Interactive & scriptable**
  - `-b` → fuzzy picker
  - `-O` → open editor at the matching line
  - `-f` → run a command on every result
  - `-J` / `-0` → JSON or NUL-separated output

---

### Installation

```bash
cargo install syk-search
```

---

### Quick examples

```bash
# Find TypeScript files larger than 10 kb modified in the last 2 hours
syk .ts +10kb -2h

# Search for "auth" only inside folders named "api"
syk -d api -c auth

# Interactive picker
syk -b todo

# Open matching line in your editor
syk -O "fn main"

# JSON output for scripts
syk -J -c "TODO"
```

---

### Performance

Measured 2026-11-23 · median of 15 runs · 16-thread machine

#### Name search in `/usr`

| Case                  | **syk + index** | syk   | fd    | bfs   | find   |
|-----------------------|-----------------|-------|-------|-------|--------|
| Rare name             | **11.0**        | 56.4  | 131.4 | 432.6 | 904.7  |
| Regex on names        | **14.6**        | 68.4  | 156.7 | 384.7 | 884.0  |
| Folder names          | **14.2**        | 71.6  | 162.0 | 340.9 | 1048.1 |
| Extension + size      | **25.1**        | 71.1  | 194.4 | 1023.1| 1076.4 |

#### Content search (~81 000 files in `~/.cargo/registry/src`)

| Case                      | **syk + index** | syk   | ripgrep | ugrep |
|---------------------------|-----------------|-------|---------|-------|
| Regex, all files          | **44.9**        | 121.5 | 143.1   | 298.8 |
| Regex, one thread         | **224.0**       | 729.8 | 947.8   | 1010.1|
| Whole word                | **35.3**        | 110.0 | 139.1   | 218.3 |
| Regex, `.rs` only         | **30.4**        | 73.0  | 92.3    | 161.1 |
| Two words on one line     | **56.0**        | 118.5 | 170.6   | 228.2 |

> When the index is warm, syk is usually the fastest tool in the room.  
> Without the index it still beats or matches ripgrep in most real-world cases.

---

### Exit codes

| Code | Meaning          |
|------|------------------|
| 0    | Matches found    |
| 1    | No matches       |
| 2    | Error            |

---

### License

MIT

