"""Read only: the final reconciliation of the Phase 8 mutation evidence on
the candidate (production code of 9851c24 = c90d5e5), from the sources:
faults.py at c90d5e5, the results files and the runner logs.

- PHASE1..PHASE7 (626 Linux): gate B (84), Phase 7 part 1 (23), gate C3
  (98 = 96 + MR26, FT29); what was run once at least on the candidate,
  what was not and why (expected survivor, or not selected).
- PHASE8 (Linux): part 1 (log), part 2 (results file), FT29 again in C3.
- PHASE7_WINDOWS and PHASE8_WINDOWS: on f7f8d81.
Every executed verdict is checked against the expectation of faults.py.
Usage: final-reconcile.py <p8 scratch dir>"""
import json
import os
import re
import sys

sys.path.insert(0, "<repo>/mutation")
import faults  # noqa: E402

P = sys.argv[1]
R = os.path.join(P, "results")
home = os.path.expanduser("~")
problems = []


def from_json(path):
    return {r["id"]: r for r in json.load(open(path, encoding="utf-8"))["results"]}


def from_log(path):
    out = {}
    for line in open(path, encoding="utf-8"):
        m = re.match(r"^([A-Z]+\d+)\s+(\w+)\s+(\d+)s\s+(.*)$", line.rstrip("\n"))
        if m:
            fid, verdict, seconds, rest = m.groups()
            if fid in out:
                problems.append(f"{path}: {fid} twice")
            out[fid] = {"id": fid, "verdict": verdict, "seconds": int(seconds),
                        "failed": ["(timeout)"] if rest.endswith("(timeout)") else []}
    return out


lists = {n: getattr(faults, n) for n in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6",
                                         "PHASE7", "PHASE8", "PHASE7_WINDOWS", "PHASE8_WINDOWS")}
by_id = {i["id"]: i for items in lists.values() for i in items}
universe = [i["id"] for n in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6", "PHASE7")
            for i in lists[n]]
print("lists at c90d5e5: " + ", ".join(f"{n} {len(v)}" for n, v in lists.items()))
assert len(universe) == len(set(universe))

runs = {
    "gate B (9851c24, 2026-11-09)": from_json(os.path.join(R, "results-B-9851c24.json")),
    "Phase 7 part 1 (9851c24, 2026-11-07)": from_log(f"{home}/final8-mutation-phase7-part1.log"),
    "gate C3 (c90d5e5, 2026-11-10/11)": from_json(f"{home}/diag-c3/final/results-C3-merged.json"),
    "Phase 8 part 1 (9851c24, 2026-11-05)": {k: v for k, v in from_log(f"{home}/final8-mutation-phase8.log").items()
                                             if k in {f"FT{n}" for n in range(1, 26)}},
    "Phase 8 part 2 (9851c24, 2026-11-07)": from_json(os.path.join(R, "results-phase8-9851c24-part2.json")),
    "phase7w (f7f8d81)": from_json(os.path.join(R, "results-phase7w-f7f8d81.json")),
    "phase8w (f7f8d81)": from_json(os.path.join(R, "results-phase8w-f7f8d81.json")),
}
print("\n== every run: verdicts against faults.py")
for name, results in runs.items():
    counts, timeouts, off = {}, [], []
    for fid, r in results.items():
        counts[r["verdict"]] = counts.get(r["verdict"], 0) + 1
        if fid not in by_id:
            off.append(f"{fid} unknown")
            continue
        if r["verdict"] != by_id[fid]["expect"]:
            off.append(f"{fid} {r['verdict']}, expected {by_id[fid]['expect']}")
        if "(timeout)" in r["failed"]:
            named = [n for n in r["failed"] if not n.startswith("(")]
            timeouts.append(f"{fid}{' alone' if not named else f' +{len(named)} named'}")
    print(f"{name}: {len(results)} faults, {counts}; timeouts: {timeouts or 'none'}; off: {off or 'none'}")
    problems += [f"{name}: {o}" for o in off]

linux_runs = ["gate B (9851c24, 2026-11-09)", "Phase 7 part 1 (9851c24, 2026-11-07)",
              "gate C3 (c90d5e5, 2026-11-10/11)"]
ran = set()
for name in linux_runs:
    ids = set(runs[name])
    outside = ids - set(universe) - {i["id"] for i in lists["PHASE8"]}
    if outside:
        problems.append(f"{name}: outside the lists {sorted(outside)}")
    ran |= ids & set(universe)
b, p1, c3 = (set(runs[n]) for n in linux_runs)
print("\n== PHASE1..PHASE7 (626 Linux)")
print(f"overlaps: B & part 1 {sorted(b & p1)}, B & C3 {sorted(b & c3)}, part 1 & C3 {sorted(p1 & c3)}; "
      f"C3 outside PHASE1..7: {sorted(c3 - set(universe))}")
survivors = sorted(i for i in universe if by_id[i]["expect"] != "caught")
not_run = [i for i in universe if i not in ran]
print(f"run on the candidate at least once: {len(ran)} (B {len(b)}, part 1 {len(p1)}, "
      f"C3 new {len(c3 & set(universe) - b - p1)}; MR26 in B and C3)")
print(f"not run: {len(not_run)} = {len([i for i in not_run if i in survivors])} expected survivors "
      f"{[i for i in not_run if i in survivors]} + {len([i for i in not_run if i not in survivors])} expected caught")
print(f"expected survivors in the lists: {survivors}; any of them run: {sorted(set(survivors) & ran)}")

print("\n== PHASE8 (Linux)")
p8 = {i["id"] for i in lists["PHASE8"]}
p8_ran = set(runs["Phase 8 part 1 (9851c24, 2026-11-05)"]) | set(runs["Phase 8 part 2 (9851c24, 2026-11-07)"])
print(f"{len(p8)} in the list; run on 9851c24: {len(p8_ran & p8)}; missing {sorted(p8 - p8_ran)}; "
      f"again in C3: {sorted(c3 & p8)}")
print("\n== Windows lists")
for name, key in (("phase7w (f7f8d81)", "PHASE7_WINDOWS"), ("phase8w (f7f8d81)", "PHASE8_WINDOWS")):
    ids = {i["id"] for i in lists[key]}
    print(f"{key}: {len(ids)} in the list, {len(set(runs[name]) & ids)} run on f7f8d81, "
          f"missing {sorted(ids - set(runs[name]))}")

print("\n== PROBLEMS" if problems else "\n== no problem")
for problem in problems:
    print(problem)
