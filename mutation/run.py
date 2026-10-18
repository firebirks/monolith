#!/usr/bin/env python3
"""Runs the mutation suite. See README.md in this directory.

Usage: python3 mutation/run.py <phase1|...|phase7|all> <workers> [fault id ...]

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
import signal
import subprocess
import sys
import threading
import time

import faults

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
WORK = os.path.join(HERE, "work")
# The bound of the unmodified run of a copy. The bound of the run of a
# fault is the slowest unmodified run, SLOWER times for each worker, and
# never less than TIMEOUT: a fault is counted caught by the timeout only
# when its run takes far longer than the tests take with nothing applied
# on this machine, however long that is.
UNMODIFIED_TIMEOUT = 7200
TIMEOUT = 1200
SLOWER = 2
# The fault each copy is checked with before a run, and the unit test
# that is certain to catch it.
CANARY = "X1"
CANARY_TEST = "chat_text_length_bounds"

print_lock = threading.Lock()

# The cargo processes that run now. Each runs in a process group of its
# own, so that the test binaries it starts can be ended with it.
running = set()
running_lock = threading.Lock()


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


def cargo_test(tree, crates, limit):
    command = ["cargo", "test", "--locked", "--no-fail-fast"]
    for crate in crates:
        command += ["-p", crate]
    # The screens are tested in the states their fixtures build.
    if "monolith-ui" in crates:
        command += ["--features", "monolith-ui/fixtures"]
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    process = subprocess.Popen(
        command,
        cwd=tree,
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    with running_lock:
        running.add(process)
    try:
        out, err = process.communicate(timeout=limit)
        return process.returncode, out + err, False
    except subprocess.TimeoutExpired:
        # Killing cargo alone would leave the test binary it started
        # running beside the next fault.
        kill_group(process)
        out, err = process.communicate()
        return None, (out or "") + (err or ""), True
    finally:
        with running_lock:
            running.discard(process)


def kill_group(process):
    try:
        os.killpg(process.pid, signal.SIGKILL)
    except ProcessLookupError:
        pass


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


def mutate(path, pairs):
    """Applies a fault to the file at `path` and returns what it held."""
    original = open(path).read()
    mutated = original
    for old, new in pairs:
        mutated = mutated.replace(old, new, 1)
    with open(path, "w") as handle:
        handle.write(mutated)
    if mutated == original or open(path).read() != mutated:
        log(f"{path} does not hold the fault")
        sys.exit(3)
    return original


def restore(path, original):
    """Puts back what the file at `path` held, with a time newer than any
    build of the fault, so that cargo builds it again, and checks that it
    holds exactly that. A copy that would keep running the build of a
    fault falsifies every result after it."""
    with open(path, "w") as handle:
        handle.write(original)
    os.utime(path, None)
    if open(path).read() != original:
        log(f"{path} was not restored")
        sys.exit(3)


def run_one(item, tree, limit):
    path = os.path.join(tree, item["path"])
    started = time.time()
    original = mutate(path, item["pairs"])
    try:
        code, output, timed_out = cargo_test(tree, item["crates"], limit)
    finally:
        restore(path, original)
    verdict, details = classify(code, output, timed_out)
    seeds_only = (
        verdict == "caught"
        and any("seeds" in name for name in details)
        and all("seeds" in name or name.startswith("(") for name in details)
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


def check_copy(tree):
    """Checks that a copy builds what it holds, on its own target
    directory, before any fault runs on it: the unit tests pass, fail with
    the canary fault applied, and pass again once the file is restored,
    with its crate built again. They are built for the crates of the
    unmodified run, so that the features are those of every other build of
    the copy, and only the test that catches the canary runs. Returns what
    went wrong, or None."""
    canary = next(item for item in faults.PHASE1 if item["id"] == CANARY)
    path = os.path.join(tree, canary["path"])
    crate = canary["path"].split("/")[1]
    command = ["cargo", "test", "--locked"]
    for name in faults.TESTS["monolith-identity"]:
        command += ["-p", name]
    command += ["--lib", CANARY_TEST]

    def check():
        result = subprocess.run(
            command,
            cwd=tree,
            env=dict(os.environ, CARGO_TERM_COLOR="never"),
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
        return result.returncode, result.stdout + result.stderr

    code, _ = check()
    if code != 0:
        return "the unit tests fail on the unmodified copy"
    original = mutate(path, canary["pairs"])
    try:
        code, output = check()
    finally:
        restore(path, original)
    if code == 0 or not re.search(r"^test \S+ \.\.\. FAILED", output, re.M):
        return f"{CANARY} applied was not caught: the copy may not build what it holds"
    code, output = check()
    if code != 0:
        return f"the copy fails once {CANARY} is restored: the build kept the fault"
    if f"Compiling {crate}" not in output:
        return f"{crate} was not built again once {CANARY} was restored"
    return None


def lock_work():
    """Refuses to start while another run uses the same copies: two runners
    would restore each other's files and leave trees that do not build."""
    os.makedirs(WORK, exist_ok=True)
    lock = os.path.join(WORK, "lock")
    try:
        descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    except FileExistsError:
        try:
            other = int(open(lock).read().strip() or "0")
            os.kill(other, 0)
            log(f"another run (process {other}) uses {WORK}")
            sys.exit(2)
        except (ValueError, ProcessLookupError, PermissionError):
            # The process that held the lock is gone.
            os.remove(lock)
            descriptor = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
    os.write(descriptor, str(os.getpid()).encode())
    os.close(descriptor)
    return lock


def main():
    lock = lock_work()

    def stop(number, _frame):
        # A runner that is stopped ends the tests it started and frees
        # the copies; the worker threads end with the process.
        with running_lock:
            for process in running:
                kill_group(process)
        os.remove(lock)
        os._exit(128 + number)

    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        run_main()
    finally:
        os.remove(lock)


def run_main():
    phase = sys.argv[1]
    workers = int(sys.argv[2])
    only = set(sys.argv[3:])
    selected = {
        "phase1": faults.PHASE1,
        "phase2": faults.PHASE2,
        "phase3": faults.PHASE3,
        "phase4": faults.PHASE4,
        "phase5": faults.PHASE5,
        "phase6": faults.PHASE6,
        "phase7": faults.PHASE7,
    }.get(
        phase,
        faults.PHASE1
        + faults.PHASE2
        + faults.PHASE3
        + faults.PHASE4
        + faults.PHASE5
        + faults.PHASE6
        + faults.PHASE7,
    )
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
    # charged with the whole build, check that the unmodified tree passes,
    # and measure how long its tests take.
    # The unmodified run covers the crates of the protocol and the core, as
    # it always has, and every other crate a selected fault is tested with.
    unmodified = sorted(
        set(faults.TESTS["monolith-identity"])
        | {crate for item in selected for crate in item["crates"]}
    )
    slowest = 0
    for tree in trees:
        started = time.time()
        code, output, timed_out = cargo_test(tree, unmodified, UNMODIFIED_TIMEOUT)
        if code != 0:
            log(output[-3000:])
            if timed_out:
                log(f"the unmodified tree does not end its tests in {UNMODIFIED_TIMEOUT}s")
            else:
                log("the unmodified tree does not pass its tests")
            sys.exit(3)
        slowest = max(slowest, round(time.time() - started))
        problem = check_copy(tree)
        if problem:
            log(problem)
            sys.exit(3)
    limit = max(TIMEOUT, SLOWER * workers * slowest)
    log(
        f"commit {commit}, {len(selected)} faults, {workers} workers, "
        f"unmodified run {slowest}s, timeout {limit}s"
    )

    free = list(trees)
    free_lock = threading.Lock()

    def task(item):
        with free_lock:
            tree = free.pop()
        try:
            result = run_one(item, tree, limit)
        finally:
            with free_lock:
                free.append(tree)
        note = " (seed tests only)" if result["seeds_only"] else ""
        if "(timeout)" in result["failed"]:
            note += " (timeout)"
        log(f"{result['id']:5} {result['verdict']:8} {result['seconds']:4}s  {result['what']}{note}")
        return result

    with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as pool:
        results = list(pool.map(task, selected))

    out = os.path.join(HERE, f"results-{phase}-{commit[:7]}.json")
    with open(out, "w") as handle:
        json.dump(
            {
                "commit": commit,
                "unmodified_seconds": slowest,
                "timeout": limit,
                "results": results,
            },
            handle,
            indent=1,
        )
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
