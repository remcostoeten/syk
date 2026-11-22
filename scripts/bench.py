#!/usr/bin/env python3
import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path

ABOUT = "Time syk against ripgrep, fd, find and grep on queries that print the same results."
METHOD = (
    "Every case runs each tool from the same folder with its output going into a pipe. "
    "Before anything is timed the outputs are compared as sets, and a case whose tools "
    "disagree is reported as DIFFERENT. A carriage return at the end of a line is left "
    "out of the comparison, because syk drops it and ripgrep prints it. The tools take "
    "turns within every round, which spreads background load evenly. cpu is the user and "
    "system time of the process, added over its threads, and moves less than the wall "
    "time when the machine is busy. The exit status is 1 when any case disagrees. "
    "--cold drops the file data of the code folder from memory before every timed run "
    "of a content search. It uses posix_fadvise, which needs no root. Folder listings "
    "stay in memory, so this measures reading files from disk and not listing them."
)
REGEX = r"fn \w+_unchecked\("
TODO = "TODO|FIXME"
CFG = r"cfg\("
BOTH = "pub.*unsafe|unsafe.*pub"
LIBRARY = r"^lib.*\.so"
RG = ["rg", "--no-config", "-n", "--no-heading"]
FD = ["fd", "--no-global-ignore-file"]


def code_cases():
    return [
        {
            "name": "regex in every file",
            "what": "lines matching a regular expression",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-c", REGEX],
                "rg": RG + [REGEX],
            },
        },
        {
            "name": "regex in every file, one thread",
            "what": "the same search on a single thread",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-j", "1", "-c", REGEX],
                "rg": RG + ["-j", "1", REGEX],
            },
        },
        {
            "name": "whole word, plain text",
            "what": "lines with the word transmute",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-F", "-w", "-c", "transmute"],
                "rg": RG + ["-F", "-w", "transmute"],
            },
        },
        {
            "name": "regex in .rs files only",
            "what": "TODO or FIXME in files ending in .rs",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-c", TODO, ".rs"],
                "rg": RG + [TODO, "-g", "*.rs"],
            },
        },
        {
            "name": "content in files named *build*",
            "what": "cfg( in files whose name contains build",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-n", "build", "-c", CFG],
                "rg": RG + [CFG, "-g", "*build*"],
            },
        },
        {
            "name": "content below folders named *tests*",
            "what": "assert_eq! below folders whose name contains tests",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-d", "tests", "-c", "assert_eq!"],
                "rg": RG + ["assert_eq!", "-g", "**/*tests*/**"],
            },
        },
        {
            "name": "two words on one line",
            "what": "lines with both pub and unsafe, in either order",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "pub", "unsafe", "-c"],
                "rg": RG + [BOTH],
            },
        },
        {
            "name": "matches counted per file",
            "what": "the number of lines with unsafe, per file",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-c", "unsafe", "-N"],
                "rg": ["rg", "--no-config", "-c", "unsafe"],
            },
        },
        {
            "name": "files that contain a match",
            "what": "paths of files with async fn",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-s", "-c", "async fn", "-l"],
                "rg": ["rg", "--no-config", "-l", "async fn"],
            },
        },
        {
            "name": "hidden and ignored files included",
            "what": "unsafe impl in every file that is not binary",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-a", "-c", "unsafe impl"],
                "rg": RG + ["-uu", "unsafe impl"],
                "grep": ["grep", "-rnI", "unsafe impl", "."],
            },
        },
        {
            "name": "names and contents together",
            "what": "serde_json in folder names, file names and lines",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-s", "serde_json"],
                "rg + fd": [
                    "sh",
                    "-c",
                    "rg --no-config -n --no-heading serde_json; "
                    "fd --no-global-ignore-file -s serde_json",
                ],
            },
        },
    ]


def tree_cases():
    return [
        {
            "name": "rare name in a large tree",
            "what": "entries whose name contains libssl",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-s", "-n", "libssl"],
                "fd": FD + ["-u", "-s", "libssl"],
                "find": ["find", ".", "-name", "*libssl*"],
            },
        },
        {
            "name": "regex on names",
            "what": "entries whose name starts with lib and has .so in it",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-s", "-n", LIBRARY],
                "fd": FD + ["-u", "-s", LIBRARY],
                "find": ["find", ".", "-name", "lib*.so*"],
            },
        },
        {
            "name": "folder names only",
            "what": "folders whose name contains python",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-s", "-d", "python"],
                "fd": FD + ["-u", "-s", "-t", "d", "python"],
                "find": ["find", ".", "-type", "d", "-name", "*python*"],
            },
        },
        {
            "name": "extension and size",
            "what": ".so files of 1 MiB or more",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-t", "file", ".so", "+1mb"],
                "fd": FD + ["-u", "-t", "f", "-e", "so", "-S", "+1mi"],
                "find": ["find", ".", "-type", "f", "-name", "*.so", "-size", "+1048575c"],
            },
        },
        {
            "name": "extension and age",
            "what": ".py files changed in the last 30 days",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-t", "file", ".py", "-30d"],
                "fd": FD + ["-u", "-t", "f", "-e", "py", "--changed-within", "30d"],
                "find": ["find", ".", "-type", "f", "-name", "*.py", "-mtime", "-30"],
            },
        },
        {
            "name": "extension, size and age",
            "what": ".h files of 100 KiB or more, older than a year",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-a", "-t", "file", ".h", "+100kb", "+365d"],
                "fd": FD
                + ["-u", "-t", "f", "-e", "h", "-S", "+100ki", "--changed-before", "365d"],
                "find": [
                    "find",
                    ".",
                    "-type",
                    "f",
                    "-name",
                    "*.h",
                    "-size",
                    "+102399c",
                    "-mtime",
                    "+364",
                ],
            },
        },
    ]


def small_cases():
    return [
        {
            "name": "content in a small project",
            "what": "lines with fn in the syk sources",
            "kind": "lines",
            "tools": {
                "syk": ["syk", "-U", "-s", "-c", "fn"],
                "rg": RG + ["fn"],
            },
        },
        {
            "name": "names in a small project",
            "what": "entries whose name contains walk",
            "kind": "paths",
            "tools": {
                "syk": ["syk", "-U", "-s", "-n", "walk"],
                "fd": FD + ["-s", "walk"],
            },
        },
    ]


def forget(folder):
    for path, _, names in os.walk(folder):
        for name in names:
            try:
                handle = os.open(os.path.join(path, name), os.O_RDONLY | os.O_NOFOLLOW)
            except OSError:
                continue
            os.posix_fadvise(handle, 0, 0, os.POSIX_FADV_DONTNEED)
            os.close(handle)


def resolve(argv, syk):
    if argv[0] == "syk":
        return [syk] + argv[1:]
    if argv[0] == "sh":
        return argv
    found = shutil.which(argv[0])
    return [found] + argv[1:] if found else None


def run(argv, folder):
    started = time.perf_counter()
    child = subprocess.Popen(
        argv,
        cwd=folder,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    output = child.stdout.read()
    usage = os.wait4(child.pid, 0)[2]
    child.returncode = 0
    return time.perf_counter() - started, output, usage.ru_utime + usage.ru_stime


def normal(output, kind):
    rows = set()
    for row in output.split(b"\n"):
        if not row:
            continue
        row = row.rstrip(b"\r")
        if row.startswith(b"./"):
            row = row[2:]
        if kind == "paths":
            row = row.rstrip(b"/")
        rows.add(row)
    return rows


def agreement(outputs, kind):
    sets = {tool: normal(output, kind) for tool, output in outputs.items()}
    first, *rest = sets.items()
    differing = [tool for tool, rows in rest if rows != first[1]]
    if not differing:
        return True, f"identical, {len(first[1])} results"
    sizes = ", ".join(f"{tool} {len(rows)}" for tool, rows in sets.items())
    return False, f"DIFFERENT: {sizes}"


def measure(case, folder, syk, runs, warmup, cold):
    tools = {}
    for tool, argv in case["tools"].items():
        resolved = resolve(argv, syk)
        if resolved:
            tools[tool] = resolved
    if len(tools) < 2:
        return None
    outputs = {}
    for _ in range(warmup):
        for tool, argv in tools.items():
            outputs[tool] = run(argv, folder)[1]
    same, note = agreement(outputs, case["kind"])
    times = {tool: [] for tool in tools}
    work = {tool: [] for tool in tools}
    order = list(tools)
    for round in range(runs):
        turn = order[round % len(order) :] + order[: round % len(order)]
        for tool in turn:
            if cold:
                forget(folder)
            taken, _, cpu = run(tools[tool], folder)
            times[tool].append(taken)
            work[tool].append(cpu)
    rows = {
        tool: {
            "min": min(taken),
            "median": statistics.median(taken),
            "mean": statistics.fmean(taken),
            "stdev": statistics.stdev(taken) if len(taken) > 1 else 0.0,
            "cpu": statistics.median(work[tool]),
            "command": " ".join(case["tools"][tool]),
        }
        for tool, taken in times.items()
    }
    return {
        "name": case["name"],
        "what": case["what"],
        "same": same,
        "note": note,
        "tools": rows,
    }


def report(result):
    print(f"\n{result['name']}: {result['what']}")
    print(f"  results  {result['note']}")
    best = min(row["median"] for row in result["tools"].values())
    for tool, row in result["tools"].items():
        print(
            f"  {tool:<8} median {row['median'] * 1000:8.1f} ms"
            f"   min {row['min'] * 1000:8.1f} ms"
            f"   sd {row['stdev'] * 1000:6.1f} ms"
            f"   cpu {row['cpu'] * 1000:8.1f} ms"
            f"   {row['median'] / best:5.2f}x"
        )


def version(argv):
    found = shutil.which(argv[0]) or argv[0]
    try:
        done = subprocess.run([found] + argv[1:], capture_output=True, text=True)
    except OSError:
        return "not installed"
    lines = done.stdout.splitlines()
    return lines[0] if lines else "unknown"


def main():
    here = Path(__file__).resolve().parent.parent
    built = here / "target" / "release" / "syk"
    parser = argparse.ArgumentParser(description=ABOUT, epilog=METHOD)
    parser.add_argument("--code", type=Path, default=Path.home() / ".cargo/registry/src")
    parser.add_argument("--tree", type=Path, default=Path("/usr"))
    parser.add_argument("--syk", default=str(built) if built.exists() else "syk")
    parser.add_argument("--runs", type=int, default=15)
    parser.add_argument("--warmup", type=int, default=2)
    parser.add_argument("--small", type=Path, default=here)
    parser.add_argument("--cold", action="store_true")
    parser.add_argument("--json", type=Path)
    args = parser.parse_args()

    syk = shutil.which(args.syk)
    if not syk:
        sys.exit(f"bench: {args.syk} is not an executable.")
    for folder in (args.code, args.tree, args.small):
        if not folder.is_dir():
            sys.exit(f"bench: {folder} is not a folder.")

    setup = {
        "syk": version([syk, "-V"]),
        "rg": version(["rg", "--version"]),
        "fd": version(["fd", "--version"]),
        "find": version(["find", "--version"]),
        "grep": version(["grep", "--version"]),
        "cores": os.cpu_count(),
        "load": os.getloadavg()[0],
        "runs": args.runs,
        "code": str(args.code),
        "tree": str(args.tree),
        "small": str(args.small),
        "cold": args.cold,
    }
    for key, value in setup.items():
        print(f"{key:<6} {value}")

    results = []
    groups = (
        (args.code, code_cases(), args.cold),
        (args.tree, tree_cases(), False),
        (args.small, small_cases(), False),
    )
    for folder, cases, cold in groups:
        for case in cases:
            result = measure(case, folder, syk, args.runs, args.warmup, cold)
            if result:
                result["folder"] = str(folder)
                report(result)
                results.append(result)

    if args.json:
        args.json.write_text(json.dumps({"setup": setup, "results": results}, indent=2))
    if not all(result["same"] for result in results):
        sys.exit(1)


if __name__ == "__main__":
    main()
