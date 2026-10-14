"""Tallies the outcome of every fault of the final verification of 91bf22c
from the runner logs (every segment has its lines) and compares it with
the expectation of mutation/faults.py."""
import collections
import glob
import os
import re
import sys

sys.path.insert(0, os.path.expanduser("~/mrun/mutation"))
import faults  # noqa: E402

lists = {"phase1": faults.PHASE1, "phase2": faults.PHASE2, "phase3": faults.PHASE3,
         "phase4": faults.PHASE4, "phase5": faults.PHASE5}
line = re.compile(r"^([A-Z]+[0-9]+) +(caught|survived|invalid) +([0-9]+)s  (.*)$", re.M)
for phase, items in lists.items():
    seen = {}
    for log in sorted(glob.glob(os.path.expanduser(f"~/final-mutation-{phase}*.log"))):
        for m in line.finditer(open(log).read()):
            if m.group(1) in seen:
                print(f"{phase}: {m.group(1)} twice")
            seen[m.group(1)] = (m.group(2), int(m.group(3)), m.group(4).endswith("(timeout)"), os.path.basename(log))
    ids = [i["id"] for i in items]
    missing = [i for i in ids if i not in seen]
    extra = [i for i in seen if i not in ids]
    counts = collections.Counter(v[0] for v in seen.values())
    expect = {i["id"]: i["expect"] for i in items}
    unexpected = [i for i in ids if i in seen and (seen[i][0] == "caught") != (expect[i] == "caught")]
    timeouts = [i for i in ids if i in seen and seen[i][2]]
    print(phase, len(ids), dict(counts), "missing", missing, "extra", extra,
          "not as expected", [(i, seen[i][0], expect[i]) for i in unexpected], "timeout", timeouts,
          "expectations", dict(collections.Counter(expect.values())))
