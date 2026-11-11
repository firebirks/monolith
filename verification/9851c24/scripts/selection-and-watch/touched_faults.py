"""Read-only analysis: which faults of Phases 1 to 7 sit in a file that
changed between two commits, or were caught (in their last evidence) only
by tests whose file changed, and what running them would cost.
Usage: touched_faults.py <repo> <old> <new> <results dir> <done log>"""
import json
import os
import re
import subprocess
import sys

repo, old, new, folder, done_log = sys.argv[1:6]
sys.path.insert(0, os.path.join(repo, "mutation"))
import faults  # noqa: E402

changed = set(subprocess.run(["git", "-C", repo, "diff", "--name-only", old, new],
                             check=True, capture_output=True, text=True).stdout.split())

# Where each test name lives at the new commit.
where = {}
attr = re.compile(r"#\[(?:tokio::)?test(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+([a-z_0-9]+)")
files = subprocess.run(["git", "-C", repo, "ls-tree", "-r", "--name-only", new], check=True,
                       capture_output=True, text=True).stdout.split()
for path in files:
    if not path.endswith(".rs"):
        continue
    text = subprocess.run(["git", "-C", repo, "show", f"{new}:{path}"], capture_output=True,
                          text=True, encoding="utf-8", errors="replace").stdout
    for m in attr.finditer(text):
        where.setdefault(m.group(1), set()).add(path)
    for m in re.finditer(r"proptest!\s*\{", text):
        for fn in re.finditer(r"\bfn\s+([a-z_0-9]+)\s*\(", text[m.end():m.end() + 20000]):
            where.setdefault(fn.group(1), set()).add(path)

evidence = {}
for name in sorted(os.listdir(folder)):
    m = re.match(r"results-phase([1-7])-(740b671|bbf1279)\.json$", name)
    if not m:
        continue
    for r in json.load(open(os.path.join(folder, name), encoding="utf-8"))["results"]:
        # bbf1279 sorts after 740b671: its evidence wins.
        evidence[r["id"]] = r

done = set(re.findall(r"^([A-Z]+[0-9]+) +caught", open(done_log, encoding="utf-8").read(), re.M))
ratio, workers = 1.68, 2
total = {"file": 0, "tests": 0}
count = {"file": [], "tests": []}
by_prefix = {}
for phase in range(1, 8):
    for f in getattr(faults, f"PHASE{phase}"):
        if f["id"] in done:
            continue
        ev = evidence.get(f["id"])
        seconds = (ev["seconds"] if ev else 900) * ratio / workers
        if f["path"] in changed:
            count["file"].append(f["id"])
            total["file"] += seconds
            prefix = re.match(r"[A-Z]+", f["id"]).group(0)
            group = by_prefix.setdefault(prefix, [0, 0.0, set()])
            group[0] += 1
            group[1] += seconds
            group[2].add(f["path"])
            continue
        names = [n.split("::")[-1] for n in (ev["failed"] if ev else []) if not n.startswith("(")]
        if ev and names:
            paths = set().union(*(where.get(n, {"?"}) for n in names))
            if paths and all(p in changed or p == "?" for p in paths):
                count["tests"].append(f["id"])
                total["tests"] += seconds
print(f"files changed {old}..{new}: {len(changed)}")
print(f"faults left (not run on {new}): {sum(len(getattr(faults, f'PHASE{p}')) for p in range(1, 8)) - len(done)}")
print(f"in a changed file: {len(count['file'])}, about {total['file'] / 3600:.1f} h")
print("   " + " ".join(count["file"]))
for prefix, (n, seconds, paths) in sorted(by_prefix.items()):
    print(f"   {prefix}: {n} faults, about {seconds / 3600:.1f} h, in {', '.join(sorted(p.replace('crates/', '') for p in paths))}")
print(f"caught only by tests in changed files: {len(count['tests'])}, about {total['tests'] / 3600:.1f} h")
print("   " + " ".join(count["tests"]))
