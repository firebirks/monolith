"""Read only: a trial of validate-c2.py on the faults of gate C3 that have
ended, before the runner writes its results. Rebuilds their records from
the runner's log lines and the whole output of each run, with the runner's
own classify; writes them, and the ids, to a scratch directory.
Usage: validate-c3-dry.py <runner log> <logs dir> <out dir>"""
import json
import os
import re
import sys

sys.path.insert(0, "<repo>/mutation")
import faults  # noqa: E402
import run  # noqa: E402

runner_log, logs, out = sys.argv[1:4]
entry = {item["id"]: item for name in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5",
                                       "PHASE6", "PHASE7", "PHASE8")
         for item in getattr(faults, name)}
records = []
for line in open(runner_log, encoding="utf-8"):
    match = re.match(r"^([A-Z]+\d+)\s+(\w+)\s+(\d+)s\s+(.*)$", line.rstrip("\n"))
    if not match:
        continue
    fid, verdict, seconds, rest = match.groups()
    timed_out = rest.endswith("(timeout)")
    logged = os.path.join(logs, fid)
    again, details = run.classify(None if timed_out else 1, run.output(logged), timed_out)
    item = entry[fid]
    records.append({"id": fid, "path": item["path"], "what": item["what"], "group": item["group"],
                    "expect": item["expect"], "verdict": verdict, "seeds_only": False,
                    "failed": details, "seconds": int(seconds), "reclassified": again})
    if again != verdict:
        print(f"{fid}: the log says {verdict}, its output classifies {again}")
os.makedirs(out, exist_ok=True)
json.dump({"commit": "c90d5e5 (partial)", "unmodified_seconds": 803, "timeout": 3212,
           "results": records}, open(os.path.join(out, "partial.json"), "w"), indent=1)
open(os.path.join(out, "ids.txt"), "w").write("\n".join(r["id"] for r in records) + "\n")
print(f"{len(records)} faults rebuilt")
