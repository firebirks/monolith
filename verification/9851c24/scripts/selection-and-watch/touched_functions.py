"""Read-only analysis: the faults of Phases 1 to 7 whose mutated text lies
in a function that the change between two commits edited (a hunk of
`git diff -U0` inside the span from the `fn` before the fault to the next
`fn`), with the cost of running them.
Usage: touched_functions.py <repo> <old> <new> <results dir> <done log>"""
import json
import os
import re
import subprocess
import sys

repo, old, new, folder, done_log = sys.argv[1:6]
sys.path.insert(0, os.path.join(repo, "mutation"))
import faults  # noqa: E402

FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?fn\s+\w+")


def show(path):
    return subprocess.run(["git", "-C", repo, "show", f"{new}:{path}"], capture_output=True,
                          text=True, encoding="utf-8", errors="replace").stdout


def hunks(path):
    out = subprocess.run(["git", "-C", repo, "diff", "-U0", old, new, "--", path],
                         capture_output=True, text=True, encoding="utf-8", errors="replace").stdout
    spans = []
    for m in re.finditer(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", out, re.M):
        start, length = int(m.group(1)), int(m.group(2) or 1)
        spans.append((start, start + max(length, 1) - 1))
    return spans


evidence = {}
for name in sorted(os.listdir(folder)):
    if re.match(r"results-phase[1-7]-(740b671|bbf1279)\.json$", name):
        for r in json.load(open(os.path.join(folder, name), encoding="utf-8"))["results"]:
            evidence[r["id"]] = r
done = set(re.findall(r"^([A-Z]+[0-9]+) +caught", open(done_log, encoding="utf-8").read(), re.M))
cache = {}
picked, cost, groups = [], 0.0, {}
for phase in range(1, 8):
    for f in getattr(faults, f"PHASE{phase}"):
        if f["id"] in done:
            continue
        path = f["path"]
        if path not in cache:
            spans = hunks(path)
            cache[path] = (show(path).splitlines(), spans)
        lines, spans = cache[path]
        if not spans:
            continue
        text = "\n".join(lines)
        old_text = f["pairs"][0][0]
        at = text.find(old_text)
        if at < 0:
            continue
        line = text.count("\n", 0, at) + 1
        begin = next((i + 1 for i in range(line - 1, -1, -1) if FN.match(lines[i])), 1)
        end = next((i for i in range(line, len(lines)) if FN.match(lines[i])), len(lines))
        if any(a <= end and b >= begin for a, b in spans):
            ev = evidence.get(f["id"])
            seconds = (ev["seconds"] if ev else 900) * 1.68 / 2
            picked.append(f["id"])
            cost += seconds
            prefix = re.match(r"[A-Z]+", f["id"]).group(0)
            g = groups.setdefault(prefix, [0, 0.0])
            g[0] += 1
            g[1] += seconds
print(f"faults in a function the change edited: {len(picked)}, about {cost / 3600:.1f} h with 2 workers")
for prefix, (n, seconds) in sorted(groups.items()):
    print(f"   {prefix}: {n}, about {seconds / 3600:.1f} h")
print("   " + " ".join(picked))
