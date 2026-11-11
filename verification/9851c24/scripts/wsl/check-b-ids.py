import re
import sys

script = open(sys.argv[1]).read()
ids = re.search(r'^ids="(.*)"$', script, re.M).group(1).split()
sys.path.insert(0, "mutation")
import faults

known = {}
for name in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6", "PHASE7"):
    for item in getattr(faults, name):
        known[item["id"]] = (name, item)
print("ids", len(ids), "unique", len(set(ids)), "unknown", [i for i in ids if i not in known])
by = {}
for i in ids:
    by[known[i][0]] = by.get(known[i][0], 0) + 1
print(by)
print(sorted({c for i in ids for c in known[i][1]["crates"]}))
