#!/usr/bin/env python3
"""Selects the faults a change can affect. See README.md in this directory,
"Incremental runs".

Usage: python3 mutation/affected.py <candidate> <results file> [...]

Each results file is a `results-<list>-<commit>.json` of run.py, which
names the commit it ran on. For each fault the last file that has it gives
its evidence: its outcome, the tests
that failed, and the commit it was run on. The fault runs again when the
change from that commit to the candidate can alter its outcome, when no
file has it, or when its entry in faults.py differs. The output gives,
for each list, the faults to run again with the reason of each, and the
run.py commands.

The exit code is 0, or 2 when the arguments are wrong.
"""

import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)

LISTS = {
    "phase1": "PHASE1",
    "phase2": "PHASE2",
    "phase3": "PHASE3",
    "phase4": "PHASE4",
    "phase5": "PHASE5",
    "phase6": "PHASE6",
    "phase7": "PHASE7",
    "phase7w": "PHASE7_WINDOWS",
}

# Files that change what any test builds or how the runner runs: every
# fault whose evidence is older runs again.
EVERYTHING = re.compile(
    r"^(Cargo\.toml|Cargo\.lock|rust-toolchain(\.toml)?|\.cargo/config(\.toml)?"
    r"|crates/[^/]+/Cargo\.toml|crates/[^/]+/build\.rs|mutation/run\.py)$"
)

# Files no test of the mutation suite builds or reads.
NOTHING = re.compile(
    r"^(docs/|\.github/|integrations/|packaging/|verification/|tests/"
    r"|fuzz/(?!seeds/)|[^/]+\.md$|LICENSE$|deny\.toml$|clippy\.toml$"
    r"|\.cargo/audit\.toml$|\.gitignore$|mutation/README\.md$|mutation/affected\.py$)"
)

# Tests that read files outside their own crate. A test that starts to
# read another file is added here, or a change to that file is missed.
# Each entry: a pattern of the files read, and the crate and the file of
# the tests that read them.
READERS = [
    # The production sources of every crate.
    (re.compile(r"^crates/[^/]+/src/"), "monolith-core", "crates/monolith-core/tests/structure.rs"),
    (re.compile(r"^docs/PROTOCOL\.md$"), "monolith-session", "crates/monolith-session/src/tests/vectors.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-core", "crates/monolith-core/tests/fuzz_seeds.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-protocol", "crates/monolith-protocol/tests/fuzz_seeds.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-session", "crates/monolith-session/src/tests/seeds.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-storage", "crates/monolith-storage/tests/fuzz_seeds.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-tor", "crates/monolith-tor/tests/fuzz_seeds.rs"),
    (re.compile(r"^fuzz/seeds/"), "monolith-ui-model", "crates/monolith-ui-model/tests/fuzz_seeds.rs"),
]


def git(*args):
    return subprocess.run(
        ["git", "-C", REPO, *args], check=True, capture_output=True, text=True,
        encoding="utf-8",
    ).stdout


def show(commit, path):
    """The text of `path` at `commit`, or "" where it does not exist."""
    run = subprocess.run(
        ["git", "-C", REPO, "show", f"{commit}:{path}"], capture_output=True,
        text=True, encoding="utf-8", errors="replace",
    )
    return run.stdout if run.returncode == 0 else ""


def fault_lists(commit):
    """The fault entries of faults.py at `commit`, by list and by id."""
    namespace = {}
    exec(compile(show(commit, "mutation/faults.py"), "faults.py", "exec"), namespace)
    by_list = {name: namespace.get(var, []) for name, var in LISTS.items()}
    by_id = {item["id"]: item for items in by_list.values() for item in items}
    return by_list, by_id


def dependencies():
    """Each workspace crate with every workspace crate it builds with,
    dev-dependencies included, itself among them."""
    metadata = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--no-deps", "--offline"],
        cwd=REPO, check=True, capture_output=True, text=True, encoding="utf-8",
    ).stdout)
    members = {package["name"] for package in metadata["packages"]}
    direct = {
        package["name"]: {d["name"] for d in package["dependencies"] if d["name"] in members}
        for package in metadata["packages"]
    }
    closure = {}
    for crate in members:
        seen, pending = {crate}, [crate]
        while pending:
            for dependency in direct.get(pending.pop(), ()):
                if dependency not in seen:
                    seen.add(dependency)
                    pending.append(dependency)
        closure[crate] = seen
    return closure


def test_names(commit, path):
    """Every function name in the test file `path` at `commit`. Helpers are
    counted with the tests, which only widens the selection."""
    return set(re.findall(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", show(commit, path)))


def tests_of(commits, crate):
    """Every function name of the integration tests of `crate`, at any of
    `commits`."""
    names = set()
    for commit in commits:
        listing = git("ls-tree", "-r", "--name-only", commit, f"crates/{crate}/tests/")
        for path in listing.split():
            if path.endswith(".rs"):
                names |= test_names(commit, path)
    return names


def effects(base, candidate):
    """What the change from `base` to `candidate` can alter, as a list of
    ("everything", path), ("code", crate, path) and
    ("tests", crate, names or None, path)."""
    found = []
    changed = git("diff", "--name-only", "--no-renames", base, candidate).split()
    commits = (base, candidate)
    for path in changed:
        if EVERYTHING.match(path):
            found.append(("everything", path))
            continue
        read = False
        for pattern, crate, reader in READERS:
            if pattern.match(path):
                read = True
                names = test_names(base, reader) | test_names(candidate, reader)
                found.append(("tests", crate, names, path))
        if path == "mutation/faults.py":
            # Entries that differ are compared one by one.
            continue
        match = re.match(r"^crates/([^/]+)/(.*)$", path)
        if match:
            crate, rest = match.groups()
            test_file = re.match(r"^tests/([^/]+\.rs)$", rest)
            if test_file:
                names = test_names(base, path) | test_names(candidate, path)
                found.append(("tests", crate, names, path))
            elif rest.startswith("tests/"):
                # A module the test files share.
                found.append(("tests", crate, tests_of(commits, crate), path))
            else:
                # Sources, examples (cargo test builds them), assets and
                # proptest regressions.
                found.append(("code", crate, path))
            continue
        if read or NOTHING.match(path):
            continue
        found.append(("everything", path))
    return found


def reason(item, evidence, change, closure):
    """Why the change can alter the outcome of `item`, or None."""
    tested = set(item["crates"])
    built = set().union(*(closure.get(crate, {crate}) for crate in tested))
    if change[0] == "everything":
        return f"{change[1]} changed"
    if change[0] == "code":
        if change[1] in built:
            return f"{change[2]} is built with the tests of the fault"
        return None
    _, crate, names, path = change
    if crate not in tested:
        return None
    failed = evidence.get("failed", [])
    if evidence.get("verdict") != "caught":
        return f"{path} changed and the fault {evidence.get('verdict')}"
    if any(name.startswith("(") for name in failed):
        return f"{path} changed and the fault was caught by {failed[0]}"
    if names is None:
        return f"{path} changed"
    hit = sorted({name.rsplit("::", 1)[-1] for name in failed} & names)
    if hit:
        return f"{path} holds {', '.join(hit)}, which caught the fault"
    return None


def main():
    if len(sys.argv) < 2:
        print(__doc__.strip(), file=sys.stderr)
        sys.exit(2)
    candidate = git("rev-parse", sys.argv[1]).strip()
    evidence = {}
    for path in sys.argv[2:]:
        with open(path, encoding="utf-8") as file:
            data = json.load(file)
        commit = git("rev-parse", data["commit"]).strip()
        for result in data["results"]:
            evidence[result["id"]] = dict(result, commit=commit)
    by_list, current = fault_lists(candidate)
    closure = dependencies()
    changes, entries = {}, {}
    print(f"candidate {candidate}")
    for name, items in by_list.items():
        again = []
        for item in items:
            held = evidence.get(item["id"])
            if held is None:
                why = "no result"
            else:
                commit = held["commit"]
                if commit not in changes:
                    changes[commit] = effects(commit, candidate)
                    entries[commit] = fault_lists(commit)[1]
                if entries[commit].get(item["id"]) != item:
                    why = "its entry in faults.py changed"
                else:
                    reasons = [r for r in (reason(item, held, c, closure) for c in changes[commit]) if r]
                    why = reasons[0] if reasons else None
            if why:
                again.append((item["id"], why))
        print(f"{name}: {len(again)} of {len(items)} faults run again")
        for fid, why in again:
            print(f"  {fid}: {why}")
        if again:
            print(f"  python3 mutation/run.py {name} <workers> {' '.join(fid for fid, _ in again)}")


if __name__ == "__main__":
    main()
