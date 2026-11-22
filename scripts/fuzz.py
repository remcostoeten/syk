import os, random, shutil, subprocess, sys, time
from pathlib import Path

SYK = sys.argv[1]
BASE = Path(sys.argv[2])
SEED = int(sys.argv[3])
TREES = int(sys.argv[4])
ROUNDS = int(sys.argv[5])
random.seed(SEED)
ENV = dict(os.environ, SYK_CONFIG="/nonexistent", XDG_CACHE_HOME=str(BASE / "cache"), NO_COLOR="1")
NAMES = ["alpha", "beta", "gamma", "delta", "node_modules", "build", "notes.txt", "main.rs", "data.log",
         "Cargo.lock", "x.pyc", "lib", "src", "docs", "a b", "needle.md", "deep", "tmp", ".hidden", "zeta.rs"]
used = {"index": 0, "live": 0, "reread": 0}


def run(root, args, extra=()):
    done = subprocess.run([SYK, *args, *extra, "-T", "-p"], cwd=root, env=ENV, stdin=subprocess.DEVNULL,
                          capture_output=True)
    lines = done.stdout.split(b"\n")
    if "-o" not in args:
        lines = sorted(lines)
    import re
    errors = sorted(re.sub(rb"IO error for operation on [^:]*: ", b"", line) for line in done.stderr.split(b"\n")
                    if line and b"matched. Read" not in line)
    stats = [line for line in done.stderr.split(b"\n") if b"matched. Read" in line]
    return lines, errors, done.returncode, stats[0] if stats else b""


def folders(root):
    found = [root]
    for path, dirs, _ in os.walk(root):
        found.extend(Path(path) / name for name in dirs if not (Path(path) / name).is_symlink())
    return found


def files(root):
    return [Path(path) / name for path, _, names in os.walk(root) for name in names]


def write(path):
    path.write_text(random.choice(["needle here\n", "nothing\n", "a needle\nand more\n"]))


def mutate(root):
    kind = random.choice(["file", "file", "file", "unlink", "unlink", "mkdir", "rmtree", "rename", "rename",
                          "swap", "ignore", "unignore", "chmod", "tree", "symlink", "touch"])
    where = random.choice(folders(root))
    name = random.choice(NAMES)
    target = where / name
    try:
        if kind == "file" and not target.exists():
            write(target)
        elif kind == "unlink":
            found = files(root)
            if found:
                random.choice(found).unlink()
        elif kind == "mkdir" and not target.exists():
            target.mkdir()
        elif kind == "tree" and not target.exists():
            target.mkdir()
            for inner in random.sample(NAMES, 4):
                if "." in inner:
                    write(target / inner)
                else:
                    (target / inner).mkdir()
                    write(target / inner / random.choice(NAMES))
        elif kind == "rmtree":
            found = [path for path in folders(root) if path != root]
            if found:
                shutil.rmtree(random.choice(found), ignore_errors=True)
        elif kind == "rename":
            found = [path for path in folders(root) + files(root) if path != root]
            if found:
                source = random.choice(found)
                if not target.exists() and source not in target.parents and source.exists():
                    source.rename(target)
        elif kind == "swap":
            found = files(root)
            if found:
                chosen = random.choice(found)
                chosen.unlink()
                chosen.mkdir()
                write(chosen / "inner.txt")
        elif kind == "ignore":
            (where / random.choice([".gitignore", ".ignore"])).write_text(
                random.choice(["*.log\n", "build\n", "docs/\n", "!notes.txt\n*.txt\n", "/lib\n", "deep\n", "tmp/\n*.rs\n"]))
        elif kind == "unignore":
            found = [path for path in files(root) if path.name in (".gitignore", ".ignore")]
            if found:
                random.choice(found).unlink()
        elif kind == "chmod":
            found = [path for path in folders(root) if path != root]
            if found:
                chosen = random.choice(found)
                chosen.chmod(0o000 if random.random() < 0.5 else 0o755)
        elif kind == "symlink" and not target.exists():
            target.symlink_to(random.choice(folders(root)))
        elif kind == "touch":
            found = files(root)
            if found:
                write(random.choice(found))
    except (OSError, shutil.Error):
        pass
    return kind


def restore(root):
    for path, dirs, _ in os.walk(root):
        for name in dirs:
            try:
                (Path(path) / name).chmod(0o755)
            except OSError:
                pass


QUERIES = [["-n"], ["-d"], ["-c", "needle", "-l"], ["-n", "a"], ["-n", "-E", "docs"], ["-n", "-D", "2"],
           ["needle"], ["-n", "rs", "-c", "needle"], ["-n", "-g", "build"], ["-t", "file", ".rs"], ["-n", "-o"],
           ["-n", "^[a-d]"], ["-n", "s$", "-o"], ["-d", "a", "-E", "lib"], ["-n", "e", "-D", "3"], ["-n", "*.rs"],
           ["-d", "t", "-n", "a"], ["-n", "Cargo"], ["-n", "a", "b"]]


def check(root, label):
    for query in QUERIES:
        indexed = run(root, query)
        live = run(root, query, ["-Y"])
        if b"from the index" in indexed[3]:
            used["index"] += 1
            used["reread"] += b"read again" in indexed[3]
        else:
            used["live"] += 1
        if indexed[:3] != live[:3]:
            print(f"MISMATCH seed {SEED} {label} query {query} in {root}")
            print("  only index:", [line for line in indexed[0] if line not in live[0]][:10])
            print("  only live: ", [line for line in live[0] if line not in indexed[0]][:10])
            print("  errors index:", indexed[1][:4], "live:", live[1][:4], "codes", indexed[2], live[2])
            print("  stats:", indexed[3])
            sys.exit(1)


for tree in range(TREES):
    root = BASE / f"tree{tree}"
    if root.exists():
        restore(root)
        shutil.rmtree(root)
    root.mkdir(parents=True)
    if random.random() < 0.5:
        (root / ".git").mkdir()
    for _ in range(random.randint(10, 60)):
        mutate(root)
    if random.random() < 0.7:
        time.sleep(2.1)
    built = subprocess.run([SYK, "-y"], cwd=root, env=ENV, capture_output=True, stdin=subprocess.DEVNULL)
    if built.returncode != 0:
        print("build failed", built.stderr)
        sys.exit(1)
    check(root, f"tree {tree} fresh")
    for round in range(ROUNDS):
        kinds = [mutate(root) for _ in range(random.randint(1, 6))]
        check(root, f"tree {tree} round {round} after {kinds}")
        if random.random() < 0.15:
            subprocess.run([SYK, "-y"], cwd=root, env=ENV, capture_output=True, stdin=subprocess.DEVNULL)
            check(root, f"tree {tree} round {round} rebuilt")
    restore(root)
print(f"seed {SEED}: ok, {used}")
