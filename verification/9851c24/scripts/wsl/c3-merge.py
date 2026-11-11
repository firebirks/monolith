"""Read only on the evidence: the results of gate C3 as one file, for the
validation. The 96 that ended before Windows restarted have no results
file (the runner writes it at the end): their records are rebuilt from the
runner's log line (verdict, seconds) and the whole output of the run, with
the runner's own classify, and each rebuilt verdict must equal the logged
one. FT29 and HR77 come from the runner's results file of the resumed run.
A directory of links puts the logs of the 98 side by side.
Usage: c3-merge.py <out dir>"""
import json
import os
import re
import sys

sys.path.insert(0, "<repo>/mutation")
import faults  # noqa: E402
import run  # noqa: E402

out = sys.argv[1]
first_log = os.path.expanduser("~/evidence-c3-reboot/final8-mutation-C3.log")
first_logs = "<repo>/mutation/work/logs/all-c90d5e5-20261110-151711"
second_logs = "<repo>/mutation/work/logs/all-c90d5e5-20261111-082454"
second = json.load(open("<repo>/mutation/results-all-c90d5e5.json", encoding="utf-8"))
entry = {item["id"]: item for name in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5",
                                       "PHASE6", "PHASE7", "PHASE8")
         for item in getattr(faults, name)}
problems, records = [], []
header = None
for line in open(first_log, encoding="utf-8"):
    line = line.rstrip("\n")
    if line.startswith("commit "):
        header = line
    match = re.match(r"^([A-Z]+\d+)\s+(\w+)\s+(\d+)s\s+(.*)$", line)
    if not match:
        continue
    fid, verdict, seconds, rest = match.groups()
    timed_out = rest.endswith("(timeout)")
    logged = os.path.join(first_logs, fid)
    again, details = run.classify(None if timed_out else 1, run.output(logged), timed_out)
    if again != verdict:
        problems.append(f"{fid}: logged {verdict}, its output classifies {again}")
    item = entry[fid]
    seeds_only = (again == "caught" and any("seeds" in n for n in details)
                  and all("seeds" in n or n.startswith("(") for n in details))
    records.append({"id": fid, "path": item["path"], "what": item["what"], "group": item["group"],
                    "expect": item["expect"], "verdict": verdict, "seeds_only": bool(seeds_only),
                    "failed": details, "seconds": int(seconds), "run": "2026-11-10 (rebuilt)"})
print(f"first run: {header}; {len(records)} records rebuilt")
for r in second["results"]:
    records.append(dict(r, run="2026-11-11 (results file)"))
print(f"resumed run: commit {second['commit']}, unmodified {second['unmodified_seconds']}s, "
      f"timeout {second['timeout']}s, {len(second['results'])} results")
seen = [r["id"] for r in records]
if len(seen) != len(set(seen)):
    problems.append(f"repeated ids: {sorted(i for i in set(seen) if seen.count(i) > 1)}")
os.makedirs(out)
links = os.path.join(out, "logs")
os.makedirs(links)
for r in records:
    source = first_logs if r["run"].startswith("2026-11-10") else second_logs
    for kind in ("stdout", "stderr"):
        os.symlink(os.path.join(source, f"{r['id']}.{kind}"), os.path.join(links, f"{r['id']}.{kind}"))
json.dump({"commit": second["commit"], "unmodified_seconds": "803 (2026-11-10), 744 (2026-11-11)",
           "timeout": "3212 (2026-11-10), 2976 (2026-11-11)", "results": records},
          open(os.path.join(out, "results-C3-merged.json"), "w", encoding="utf-8"), indent=1)
for problem in problems:
    print("PROBLEM", problem)
print(f"{len(records)} records in {out}")
sys.exit(1 if problems else 0)
