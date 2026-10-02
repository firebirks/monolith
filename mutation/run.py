#!/usr/bin/env python3
"""Runs the mutation suite. See README.md in this directory.

Usage: python3 mutation/run.py <phase1|phase2|phase3|all> <workers> [fault id ...]

Each fault of faults.py is applied to a copy of the committed HEAD, one at
a time; the tests of the affected crates run; the file is restored. The
copies live under mutation/work/, one per worker, each with its own target
directory. The working tree of the repository is never modified.

The exit code is 0 when every fault has the outcome its entry expects, 1
when one does not, and 2 or 3 when the suite could not start.
"""

import concurrent.futures
import json
import os
import re
import subprocess
import sys
import threading
import time

import faults

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
WORK = os.path.join(HERE, "work")
TIMEOUT = 1200

print_lock = threading.Lock()


def log(text):
    with print_lock:
        print(text, flush=True)


def prepare(worker, commit):
    tree = os.path.join(WORK, f"tree{worker}")
    marker = os.path.join(tree, ".mutation-commit")
    if os.path.exists(marker) and open(marker).read().strip() == commit:
        return tree
    subprocess.run(["rm", "-rf", tree], check=True)
    os.makedirs(tree)
    archive = subprocess.run(
        ["git", "-C", REPO, "archive", commit], check=True, capture_output=True
    ).stdout
    subprocess.run(["tar", "-x", "-C", tree], input=archive, check=True)
    with open(marker, "w") as handle:
        handle.write(commit)
    return tree


def precheck(selected, tree):
    problems = []
    for item in selected:
        text = open(os.path.join(tree, item["path"])).read()
        for old, new in item["pairs"]:
            count = text.count(old)
            if count != 1:
                problems.append(f"{item['id']}: pattern found {count} times in {item['path']}")
            if old == new:
                problems.append(f"{item['id']}: replacement changes nothing")
            text = text.replace(old, new, 1)
    return problems


def cargo_test(tree, crates):
    command = ["cargo", "test", "--locked", "--no-fail-fast"]
    for crate in crates:
        command += ["-p", crate]
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    try:
        done = subprocess.run(
            command, cwd=tree, env=env, capture_output=True, text=True, timeout=TIMEOUT
        )
        return done.returncode, done.stdout + done.stderr, False
    except subprocess.TimeoutExpired as expired:
        out = (expired.stdout or b"") + (expired.stderr or b"")
        if isinstance(out, bytes):
            out = out.decode(errors="replace")
        return None, out, True


def classify(code, output, timed_out):
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", output, re.M)))
    if timed_out:
        return "caught", ["(timeout)"] + failed
    if code == 0:
        return "survived", []
    if failed:
        return "caught", failed
    if re.search(r"^error(\[E\d+\])?:", output, re.M) and "could not compile" in output:
        return "invalid", [line for line in output.splitlines() if line.startswith("error")][:5]
    return "caught", ["(non-zero exit, no failing test named)"]


def run_one(item, tree):
    path = os.path.join(tree, item["path"])
    original = open(path).read()
    mutated = original
    for old, new in item["pairs"]:
        mutated = mutated.replace(old, new, 1)
    started = time.time()
    try:
        with open(path, "w") as handle:
            handle.write(mutated)
        code, output, timed_out = cargo_test(tree, item["crates"])
    finally:
        with open(path, "w") as handle:
            handle.write(original)
    verdict, details = classify(code, output, timed_out)
    seeds_only = verdict == "caught" and details and all(
        "seeds" in name or name.startswith("(") for name in details
    )
    return {
        "id": item["id"],
        "path": item["path"],
        "what": item["what"],
        "group": item["group"],
        "expect": item["expect"],
        "verdict": verdict,
        "seeds_only": bool(seeds_only),
        "failed": details,
        "seconds": round(time.time() - started),
    }


def main():
    phase = sys.argv[1]
    workers = int(sys.argv[2])
    only = set(sys.argv[3:])
    selected = {
        "phase1": faults.PHASE1,
        "phase2": faults.PHASE2,
        "phase3": faults.PHASE3,
    }.get(phase, faults.PHASE1 + faults.PHASE2 + faults.PHASE3)
    if only:
        selected = [item for item in selected if item["id"] in only]
    commit = subprocess.run(
        ["git", "-C", REPO, "rev-parse", "HEAD"], check=True, capture_output=True, text=True
    ).stdout.strip()

    trees = [prepare(worker, commit) for worker in range(workers)]
    problems = precheck(selected, trees[0])
    if problems:
        for problem in problems:
            log(problem)
        sys.exit(2)

    # Build each copy once, so that the first fault of a worker is not
    # charged with the whole build, and check that the unmodified tree
    # passes.
    for tree in trees:
        code, output, _ = cargo_test(tree, faults.TESTS["monolith-identity"])
        if code != 0:
            log(output[-3000:])
            log("the unmodified tree does not pass its tests")
            sys.exit(3)
    log(f"commit {commit}, {len(selected)} faults, {workers} workers")

    free = list(trees)
    free_lock = threading.Lock()

    def task(item):
        with free_lock:
            tree = free.pop()
        try:
            result = run_one(item, tree)
        finally:
            with free_lock:
                free.append(tree)
        note = " (seed tests only)" if result["seeds_only"] else ""
        log(f"{result['id']:5} {result['verdict']:8} {result['seconds']:4}s  {result['what']}{note}")
        return result

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        results = list(pool.map(task, selected))

    out = os.path.join(HERE, f"results-{phase}-{commit[:7]}.json")
    with open(out, "w") as handle:
        json.dump({"commit": commit, "results": results}, handle, indent=1)
    counts = {}
    for result in results:
        counts[result["verdict"]] = counts.get(result["verdict"], 0) + 1
    log(f"done: {counts}; written to {out}")

    # A fault that is caught must be expected to be caught; one that
    # survives must be listed with its reason; an invalid fault is a
    # mistake in the list.
    unexpected = [
        result for result in results
        if (result["verdict"] == "caught") != (result["expect"] == "caught")
        or result["verdict"] == "invalid"
    ]
    for result in unexpected:
        log(f"UNEXPECTED {result['id']}: {result['verdict']}, expected {result['expect']}")
    sys.exit(1 if unexpected else 0)


if __name__ == "__main__":
    main()
