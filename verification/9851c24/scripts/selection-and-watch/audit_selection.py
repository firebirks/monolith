"""Read-only audit of the gate B selection (owner brief of 2026-11-09).

Evidence: the Phase 1-7 results of 740b671, then ba12472, then bbf1279;
for each fault the latest file that has it gives its evidence, as the
Phase 7 report reads them.
Universe: the Linux lists PHASE1..PHASE7 of faults.py at the candidate.
Criterion F: a pair of the fault (any pair, not only the first) lies in a
function the diff base..candidate edits (from the `fn` before the
mutated text to the next `fn`).
Criterion T: the fault is not in F and every named test of its last
evidence lies, at the candidate, in a file the diff changed (a name not
found counts as changed). Applied to every fault, in a changed file or
not.
Usage: audit_selection.py <repo> <base> <candidate> <results dir> <gate B results> <done log>"""
import json
import os
import re
import subprocess
import sys
from collections import Counter

repo, base, new, folder, b_path, done_log = sys.argv[1:7]


def git(*args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True,
                          text=True, encoding="utf-8", errors="replace").stdout


def show(commit, path):
    run = subprocess.run(["git", "-C", repo, "show", f"{commit}:{path}"], capture_output=True,
                         text=True, encoding="utf-8", errors="replace")
    return run.stdout if run.returncode == 0 else ""


def lists(commit):
    namespace = {}
    exec(compile(show(commit, "mutation/faults.py"), "faults.py", "exec"), namespace)
    return namespace


now = lists(new)
then = lists(base)
universe = [f for p in range(1, 8) for f in now[f"PHASE{p}"]]
by_id = {f["id"]: f for f in universe}
old_by_id = {f["id"]: f for p in range(1, 8) for f in then[f"PHASE{p}"]}
print(f"universe: {len(universe)} Linux faults of PHASE1..PHASE7 at {new}; "
      f"PHASE7_WINDOWS {len(now['PHASE7_WINDOWS'])} apart")

# --- A. Evidence union -------------------------------------------------------
order = {"740b671": 0, "ba12472": 1, "bbf1279": 2}
files = [n for n in os.listdir(folder) if re.match(r"results-phase[1-7]-(740b671|ba12472|bbf1279)\.json$", n)]
evidence, source, seen_in = {}, {}, {}
for name in sorted(files, key=lambda n: (order[n[-12:-5]], n)):
    data = json.load(open(os.path.join(folder, name), encoding="utf-8"))
    rev = name[-12:-5]
    assert data["commit"].startswith(rev), (name, data["commit"])
    for r in data["results"]:
        evidence[r["id"]] = r
        source[r["id"]] = rev
        seen_in.setdefault(r["id"], []).append(f"{rev}:{r['verdict']}")
print("\nA. evidence files:", ", ".join(sorted(files, key=lambda n: (order[n[-12:-5]], n))))
print("   last evidence by revision:", dict(Counter(source[f["id"]] for f in universe if f["id"] in source)))
missing = [f["id"] for f in universe if f["id"] not in evidence]
print("   faults with no evidence in the three revisions:", missing)
only_ba = [i for i in by_id if i in source and all(s.startswith("ba12472") for s in seen_in[i])]
cs_only_ba = [i for i in only_ba if i.startswith("CS")]
print(f"   evidence only in ba12472: {len(only_ba)} ({len(cs_only_ba)} CS): {' '.join(only_ba)}")
print("   MR36:", seen_in.get("MR36"), "last", source.get("MR36"), evidence.get("MR36", {}).get("failed"))


def expected(f):
    return "survived" if f["expect"].startswith("surviv") else "caught"


survivors = [f["id"] for f in universe if expected(f) == "survived"]
print("   expected survivors in faults.py:", survivors)
for i in survivors:
    print(f"     {i}: last {source.get(i)} {evidence[i]['verdict']}; all {seen_in[i]}")
off = [(f["id"], source[f["id"]], evidence[f["id"]]["verdict"], expected(f)) for f in universe
       if f["id"] in evidence and (evidence[f["id"]]["verdict"] == "caught") != (expected(f) == "caught")]
print("   last verdict not as expected:", off)
detect = Counter()
for f in universe:
    ev = evidence.get(f["id"])
    if not ev:
        continue
    named = [n for n in ev["failed"] if not n.startswith("(")]
    marks = [n for n in ev["failed"] if n.startswith("(")]
    detect[(ev["verdict"], "named" if named else "no name", " ".join(marks))] += 1
print("   last evidence by mechanism:", dict(detect))

# --- B. Selection ------------------------------------------------------------
changed = set(git("diff", "--name-only", base, new).split())
FN = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?(?:extern\s+\"[^\"]*\"\s+)?fn\s+\w+")
cache = {}


def spans(path):
    out = git("diff", "-U0", base, new, "--", path)
    result = []
    for m in re.finditer(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@", out, re.M):
        start, length = int(m.group(1)), int(m.group(2) or 1)
        result.append((start, start + max(length, 1) - 1))
    return result


def in_edited_function(f):
    path = f["path"]
    if path not in cache:
        cache[path] = (show(new, path), spans(path))
    text, hunks = cache[path]
    if not hunks:
        return False, []
    lines = text.splitlines()
    where = []
    for old_text, _ in f["pairs"]:
        at = text.find(old_text)
        if at < 0:
            where.append("pattern not found")
            continue
        first = text.count("\n", 0, at) + 1
        last = first + old_text.count("\n")
        begin = next((i + 1 for i in range(first - 1, -1, -1) if FN.match(lines[i])), 1)
        end = next((i for i in range(last, len(lines)) if FN.match(lines[i])), len(lines))
        name = FN.match(lines[begin - 1]).group(0).strip() if FN.match(lines[begin - 1] if begin - 1 < len(lines) else "") else "(top)"
        hit = any(a <= end and b >= begin for a, b in hunks)
        where.append((name, begin, end, hit))
    return any(isinstance(w, tuple) and w[3] for w in where), where


F, F_first_only, notes = set(), set(), {}
for f in universe:
    hit, where = in_edited_function(f)
    notes[f["id"]] = where
    if hit:
        F.add(f["id"])
    if where and isinstance(where[0], tuple) and where[0][3]:
        F_first_only.add(f["id"])
print(f"\nB. files changed {base}..{new}: {len(changed)}")
print(f"   faults in a changed file: {sum(f['path'] in changed for f in universe)}")
print(f"   F (any pair in an edited function): {len(F)}; with the first pair only: {len(F_first_only)}; "
      f"added by other pairs: {sorted(F - F_first_only)}")
print("   patterns not found:", [i for i, w in notes.items() if "pattern not found" in w])

test_file = {}
attr = re.compile(r"#\[(?:tokio::)?test(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+([a-z_0-9]+)")
for path in git("ls-tree", "-r", "--name-only", new).split():
    if not path.endswith(".rs"):
        continue
    text = show(new, path)
    for m in attr.finditer(text):
        test_file.setdefault(m.group(1), set()).add(path)
    for m in re.finditer(r"proptest!\s*\{", text):
        for fn in re.finditer(r"\bfn\s+([a-z_0-9]+)\s*\(", text[m.end():m.end() + 20000]):
            test_file.setdefault(fn.group(1), set()).add(path)

T, T_unchanged_file, unnamed = set(), set(), []
for f in universe:
    if f["id"] in F or f["id"] not in evidence:
        continue
    ev = evidence[f["id"]]
    named = [n.rsplit("::", 1)[-1] for n in ev["failed"] if not n.startswith("(")]
    if not named:
        unnamed.append(f["id"])
        continue
    paths = set().union(*(test_file.get(n, {"?"}) for n in named))
    if all(p in changed or p == "?" for p in paths):
        T.add(f["id"])
        if f["path"] not in changed:
            T_unchanged_file.add(f["id"])
print(f"   T (every named killing test in a changed file): {len(T)}; of them outside changed files: "
      f"{len(T_unchanged_file)} {sorted(T_unchanged_file)}; in changed files: {sorted(T - T_unchanged_file)}")
print(f"   F and T overlap: {len(F & T)} (T excludes F by construction; T computed on F as well: "
      f"{sum(1 for i in F if i in evidence and [n for n in evidence[i]['failed'] if not n.startswith('(')] and all(p in changed or p == '?' for p in set().union(*(test_file.get(n.rsplit('::', 1)[-1], {'?'}) for n in evidence[i]['failed'] if not n.startswith('(')))))})")

gate_b = json.load(open(b_path, encoding="utf-8"))
B = {r["id"] for r in gate_b["results"]}
done = set(re.findall(r"^([A-Z]+[0-9]+) +caught", open(done_log, encoding="utf-8").read(), re.M))
done &= set(by_id)
ran = B | done
S = F | T
print(f"   gate B ids {len(B)}, Phase 7 part 1 on {new[:7]} {len(done)}, overlap {len(B & done)}")
print(f"   selected S = F | T: {len(S)}")
print(f"   S already run on {new[:7]}: {len(S & ran)} (gate B {len(S & B)}, part 1 {len(S & done)})")
print(f"   S not run (newly required): {len(S - ran)} {sorted(S - ran)}")
print(f"   run but not in S: {sorted(ran - S)}")
excluded = [f["id"] for f in universe if f["id"] not in S | ran]
print(f"   excluded (not in S, not run): {len(excluded)}")
reasons = Counter()
for i in excluded:
    f, ev = by_id[i], evidence.get(i)
    named = [n for n in ev["failed"] if not n.startswith("(")] if ev else []
    if expected(f) == "survived":
        why = "expected survivor, function not edited"
    elif not named:
        why = f"function not edited, caught with no test named: {' '.join(ev['failed'])}"
    else:
        why = ("function not edited (file changed), a killing test in an unchanged file"
               if f["path"] in changed else "file unchanged, a killing test in an unchanged file")
    reasons[why] += 1
for why, n in sorted(reasons.items()):
    print(f"     {n:4} {why}")
print("   excluded with no test named:", [(i, evidence[i]["failed"], source[i]) for i in excluded
                                          if not [n for n in evidence[i]["failed"] if not n.startswith("(")]])

entry_changed = sorted(i for i in by_id if old_by_id.get(i) != by_id[i])
print(f"   entries of faults.py that differ {base}..{new}: {len(entry_changed)} {entry_changed}")
print("     of them not run on the candidate:", sorted(set(entry_changed) - ran))
win = [f["id"] for f in now["PHASE7_WINDOWS"]]
win_old = {f["id"]: f for f in then["PHASE7_WINDOWS"]}
print("   PHASE7_WINDOWS entries that differ:", [i for i in win if win_old.get(i) != {f['id']: f for f in now['PHASE7_WINDOWS']}[i]])
for i in sorted(S - ran):
    f, ev = by_id[i], evidence.get(i, {})
    print(f"   NEW {i} {f['path']} {notes[i]} last {source.get(i)} {ev.get('verdict')} {ev.get('seconds')}s {ev.get('failed')}")
