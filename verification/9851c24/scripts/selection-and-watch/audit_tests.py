"""Read-only refinement of criterion T of audit_selection.py: for the
faults it selects that did not run on the candidate, how each named
killing test of the last evidence changed from base to candidate:
same body in the same file, same body in another file, another body, or
gone. Then the cost of running them, from the seconds of their evidence
(x1.68 for the WSL machine, two workers, as the gate B estimate).
Usage: audit_tests.py <repo> <base> <candidate> <results dir> <ids...>"""
import json
import os
import re
import subprocess
import sys

repo, base, new, folder = sys.argv[1:5]
ids = sys.argv[5:]


def git(*args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True,
                          text=True, encoding="utf-8", errors="replace").stdout


texts = {}


def tree(commit):
    if commit not in texts:
        found = {}
        for path in git("ls-tree", "-r", "--name-only", commit).split():
            if path.endswith(".rs"):
                found[path] = subprocess.run(["git", "-C", repo, "show", f"{commit}:{path}"],
                                             capture_output=True, text=True, encoding="utf-8",
                                             errors="replace").stdout
        texts[commit] = found
    return texts[commit]


def bodies(commit, name):
    """Every (path, body) of a fn `name` at `commit`, the body from `fn`
    to its closing brace, whitespace runs folded."""
    out = []
    pattern = re.compile(r"\bfn\s+" + re.escape(name) + r"\s*[(<]")
    for path, text in tree(commit).items():
        for m in pattern.finditer(text):
            i = text.find("{", m.end())
            if i < 0:
                continue
            depth, j = 1, i + 1
            while depth and j < len(text):
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
                j += 1
            out.append((path, re.sub(r"\s+", " ", text[m.start():j])))
    return out


order = {"740b671": 0, "ba12472": 1, "bbf1279": 2}
evidence, source = {}, {}
names = [n for n in os.listdir(folder) if re.match(r"results-phase[1-7]-(740b671|ba12472|bbf1279)\.json$", n)]
for name in sorted(names, key=lambda n: (order[n[-12:-5]], n)):
    for r in json.load(open(os.path.join(folder, name), encoding="utf-8"))["results"]:
        evidence[r["id"]] = r
        source[r["id"]] = name[-12:-5]

strict, loose, total, strict_cost = [], [], 0.0, 0.0
for i in ids:
    ev = evidence[i]
    kinds = []
    for full in ev["failed"]:
        if full.startswith("("):
            continue
        name = full.rsplit("::", 1)[-1]
        old, now = bodies(base, name), bodies(new, name)
        if not now:
            kinds.append((name, "gone"))
        elif any(o == n for o in old for n in now):
            kinds.append((name, "same body, same file"))
        elif any(o[1] == n[1] for o in old for n in now):
            kinds.append((name, "same body, moved"))
        elif not old:
            kinds.append((name, "new name since base"))
        else:
            kinds.append((name, "body changed"))
    seconds = ev["seconds"] * 1.68 / 2
    total += seconds
    same = [k for k in kinds if k[1].startswith("same body")]
    if same:
        loose.append((i, source[i], kinds))
    else:
        strict.append((i, source[i], kinds))
        strict_cost += seconds
print(f"{len(ids)} faults, about {total / 3600:.1f} h with 2 workers, plus two unmodified runs")
print(f"\nno killing test with the same body ({len(strict)}, about {strict_cost / 3600:.1f} h):")
for i, rev, kinds in strict:
    print(f"  {i} [{rev}] " + "; ".join(f"{n}: {k}" for n, k in kinds))
print(f"\nat least one killing test with the same body ({len(loose)}):")
for i, rev, kinds in loose:
    print(f"  {i} [{rev}] " + "; ".join(f"{n}: {k}" for n, k in kinds))
