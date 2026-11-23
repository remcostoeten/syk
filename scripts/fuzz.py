#!/usr/bin/env python3
import argparse
import os
import random
import shutil
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ABOUT = "Change random trees and check that syk finds the same with an index as without one."
METHOD = (
    "Every tree gets an index from syk -y. A round then makes a few random changes: files "
    "and folders are made, removed, renamed and swapped, ignore files are written and "
    "removed, and folders lose and regain their permissions. Files also get other text, "
    "sometimes of the same length and with the old modification time put back, and some "
    "turn into binary files. A new index is first tried with changes to the text of "
    "files only, which leave every folder as it was. After every round each query "
    "runs twice, once with the index and once with -Y, and the results, the errors and "
    "the exit status have to be the same. A third run with -Y and -o walks with the other "
    "of the two folder walkers, and has to find the same again. Some queries search a "
    "folder inside the tree, which reads the index of the tree. The exit status is 1 at "
    "the first difference. "
    "Trees are made in a temporary folder, which has to be on a file system that syk "
    "builds an index for."
)
NAMES = [
    "alpha",
    "beta",
    "gamma",
    "delta",
    "node_modules",
    "build",
    "notes.txt",
    "main.rs",
    "data.log",
    "Cargo.lock",
    "x.pyc",
    "lib",
    "src",
    "docs",
    "a b",
    "needle.md",
    "deep",
    "tmp",
    ".hidden",
    "zeta.rs",
]
TEXTS = [
    "needle here\n",
    "noodle here\n",
    "nothing\n",
    "a needle\nand more\n",
    "A NEEDLE\nand less\n",
    "a noodle\nand more\n",
    "needle\0binary\n",
    "filler line without the word\n" * 400 + "the last needle\n",
]
RULES = ["*.log\n", "build\n", "docs/\n", "!notes.txt\n*.txt\n", "/lib\n", "deep\n", "tmp/\n*.rs\n"]
CHANGES = [
    "file",
    "file",
    "file",
    "unlink",
    "unlink",
    "mkdir",
    "rmtree",
    "rename",
    "rename",
    "swap",
    "ignore",
    "unignore",
    "chmod",
    "tree",
    "symlink",
    "touch",
    "touch",
    "rewrite",
    "rewrite",
]
TEXT = ["touch", "rewrite"]
QUERIES = [
    ["-n"],
    ["-d"],
    ["-c", "needle", "-l"],
    ["-n", "a"],
    ["-n", "-E", "docs"],
    ["-n", "-D", "2"],
    ["needle"],
    ["-n", "rs", "-c", "needle"],
    ["-n", "-g", "build"],
    ["-t", "file", ".rs"],
    ["-n", "-o"],
    ["-n", "^[a-d]"],
    ["-n", "s$", "-o"],
    ["-d", "a", "-E", "lib"],
    ["-n", "e", "-D", "3"],
    ["-n", "*.rs"],
    ["-d", "t", "-n", "a"],
    ["-n", "Cargo"],
    ["-n", "a", "b"],
    ["-c", "noodle"],
    ["-c", "needle", "-N"],
    ["-i", "-c", "NEEDLE", "-l"],
    ["-c", "n[eo]+dle"],
    ["-c", "and more"],
    ["-c", "last needle", "-u", "lock"],
    ["needle", "here"],
    ["-n", "lock", "-c", "needle"],
]
SETTLE = 2.1

def search(syk, env, root, query):
    done = subprocess.run(
        [syk, *query, "-T", "-p"],
        cwd=root,
        env=env,
        stdin=subprocess.DEVNULL,
        capture_output=True,
    )
    lines = done.stdout.split(b"\n")
    if "-o" not in query:
        lines = sorted(lines)
    notes = [line for line in done.stderr.split(b"\n") if line]
    totals = [line for line in notes if b"matched. Read" in line]
    errors = sorted(line for line in notes if line not in totals)
    return (lines, errors, done.returncode), totals[0] if totals else b""


def folders(root):
    found = [root]
    for path, names, _ in os.walk(root):
        found.extend(Path(path) / name for name in names if not (Path(path) / name).is_symlink())
    return found


def files(root):
    return [Path(path) / name for path, _, names in os.walk(root) for name in names]


def write(path):
    path.write_text(random.choice(TEXTS))


def change(root, kinds=CHANGES):
    kind = random.choice(kinds)
    where = random.choice(folders(root))
    target = where / random.choice(NAMES)
    below = [path for path in folders(root) if path != root]
    try:
        if kind == "file" and not target.exists():
            write(target)
        elif kind == "unlink" and files(root):
            random.choice(files(root)).unlink()
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
        elif kind == "rmtree" and below:
            shutil.rmtree(random.choice(below), ignore_errors=True)
        elif kind == "rename" and below + files(root):
            source = random.choice(below + files(root))
            if not target.exists() and source not in target.parents and source.exists():
                source.rename(target)
        elif kind == "swap" and files(root):
            chosen = random.choice(files(root))
            chosen.unlink()
            chosen.mkdir()
            write(chosen / "inner.txt")
        elif kind == "ignore":
            (where / random.choice([".gitignore", ".ignore"])).write_text(random.choice(RULES))
        elif kind == "unignore":
            rules = [path for path in files(root) if path.name in (".gitignore", ".ignore")]
            if rules:
                random.choice(rules).unlink()
        elif kind == "chmod" and below:
            random.choice(below).chmod(random.choice([0o000, 0o755]))
        elif kind == "symlink" and not target.exists():
            target.symlink_to(random.choice(folders(root)))
        elif kind == "touch" and files(root):
            write(random.choice(files(root)))
        elif kind == "rewrite" and files(root):
            chosen = random.choice(files(root))
            before = chosen.stat()
            same = [text for text in TEXTS if len(text) == before.st_size]
            chosen.write_text(random.choice(same or TEXTS))
            os.utime(chosen, ns=(before.st_atime_ns, before.st_mtime_ns))
    except (OSError, shutil.Error):
        pass
    return kind


def restore(root):
    for path, names, _ in os.walk(root):
        for name in names:
            try:
                (Path(path) / name).chmod(0o755)
            except OSError:
                pass


def inner(root):
    below = [path for path in folders(root) if path != root and os.access(path, os.R_OK | os.X_OK)]
    chosen = random.sample(below, min(3, len(below)))
    return [[f"{path.relative_to(root)}/"] for path in chosen]


def compare(syk, env, root, label, used):
    nested = [query + place for place in inner(root) for query in random.sample(QUERIES, 6)]
    for query in QUERIES + nested:
        indexed, totals = search(syk, env, root, query)
        walked, _ = search(syk, env, root, query + ["-Y"])
        if b"from the index" in totals:
            used["index"] += 1
            used["reread"] += b"read again" in totals
        else:
            used["walk"] += 1
        if indexed != walked:
            print(f"DIFFERENT {label}, query {' '.join(query)}, in {root}")
            print("  only with the index:", [line for line in indexed[0] if line not in walked[0]][:10])
            print("  only with a walk:   ", [line for line in walked[0] if line not in indexed[0]][:10])
            print("  errors:", indexed[1][:4], "against", walked[1][:4])
            print("  exit status:", indexed[2], "against", walked[2])
            return False
        if "-o" in query:
            continue
        ordered, _ = search(syk, env, root, query + ["-Y", "-o"])
        ordered = (sorted(ordered[0]), ordered[1], ordered[2])
        if ordered != walked:
            print(f"DIFFERENT {label}, query {' '.join(query)} with and without -o, in {root}")
            print("  only in path order:", [line for line in ordered[0] if line not in walked[0]][:10])
            print("  only without:      ", [line for line in walked[0] if line not in ordered[0]][:10])
            print("  errors:", ordered[1][:4], "against", walked[1][:4])
            print("  exit status:", ordered[2], "against", walked[2])
            return False
    return True


def trial(syk, env, root, rounds, used):
    root.mkdir(parents=True)
    if random.random() < 0.5:
        (root / ".git").mkdir()
    for _ in range(random.randint(10, 60)):
        change(root)
    if random.random() < 0.7:
        time.sleep(SETTLE)
    build = [syk, "-y"]
    built = subprocess.run(build, cwd=root, env=env, stdin=subprocess.DEVNULL, capture_output=True)
    if built.returncode != 0:
        print(f"no index for {root}: {built.stderr.decode().strip()}")
        return False
    if not compare(syk, env, root, "a new tree", used):
        return False
    for round in range(3):
        kinds = [change(root, TEXT) for _ in range(random.randint(1, 4))]
        if not compare(syk, env, root, f"text round {round} after {', '.join(kinds)}", used):
            return False
    for round in range(rounds):
        kinds = [change(root) for _ in range(random.randint(1, 6))]
        if not compare(syk, env, root, f"round {round} after {', '.join(kinds)}", used):
            return False
        if random.random() < 0.15:
            subprocess.run(build, cwd=root, env=env, stdin=subprocess.DEVNULL, capture_output=True)
            if not compare(syk, env, root, f"round {round} with a new index", used):
                return False
    return True


def main():
    built = Path(__file__).resolve().parent.parent / "target" / "release" / "syk"
    parser = argparse.ArgumentParser(description=ABOUT, epilog=METHOD)
    parser.add_argument("syk", nargs="?", default=str(built) if built.exists() else "syk")
    parser.add_argument("--seed", type=int, default=1)
    parser.add_argument("--trees", type=int, default=4)
    parser.add_argument("--rounds", type=int, default=10)
    args = parser.parse_args()

    syk = shutil.which(args.syk)
    if not syk:
        sys.exit(f"fuzz: {args.syk} is not an executable.")
    random.seed(args.seed)
    used = {"index": 0, "reread": 0, "walk": 0}
    base = Path(tempfile.mkdtemp(prefix="syk-fuzz-"))
    env = dict(
        os.environ,
        SYK_CONFIG="/nonexistent",
        XDG_CACHE_HOME=str(base / "cache"),
        NO_COLOR="1",
    )
    roots = [base / f"tree{tree}" for tree in range(args.trees)]
    try:
        same = all(trial(syk, env, root, args.rounds, used) for root in roots)
    finally:
        for root in roots:
            restore(root)
    print(
        f"seed {args.seed}: {'the same' if same else 'DIFFERENT'}. "
        f"{used['index']} searches used an index, {used['reread']} of them read changed "
        f"folders again, and {used['walk']} fell back to a walk."
    )
    if not same:
        sys.exit(f"fuzz: the trees are kept in {base}.")
    shutil.rmtree(base, ignore_errors=True)


if __name__ == "__main__":
    main()
