# Checks that every fault id of each list has exactly one outcome line in
# the logs of the Phase 6 final mutation run, and lists timeout catches.
# The first log of the Phase 5 list, stopped when the VM was restarted, is
# kept apart: that list was run again whole.
import collections, glob, os, re, sys
sys.path.insert(0, os.path.expanduser("~/mrun/mutation"))
import faults
home = os.path.expanduser("~")
total = 0
for n in range(6, 0, -1):
    ids = [f["id"] for f in getattr(faults, f"PHASE{n}")]
    expect = {f["id"]: f["expect"] for f in getattr(faults, f"PHASE{n}")}
    seen = collections.defaultdict(list)
    for log in sorted(glob.glob(f"{home}/final6-mutation-phase{n}*.log")):
        if log.endswith("-interrupted.log"):
            continue
        for line in open(log):
            m = re.match(r"^([A-Z]+[0-9]+) +(caught|survived|invalid) +(\d+)s +(.*)$", line.rstrip())
            if m:
                seen[m.group(1)].append((m.group(2), m.group(3), os.path.basename(log), "(timeout)" in m.group(4)))
    missing = [i for i in ids if i not in seen]
    twice = {i: v for i, v in seen.items() if len(v) != 1}
    extra = [i for i in seen if i not in expect]
    wrong = [i for i, v in seen.items() if i in expect and (v[0][0] == "caught") != (expect[i] == "caught")]
    verdicts = collections.Counter(v[0][0] for v in seen.values())
    timeouts = [i for i, v in seen.items() if v[0][3]]
    total += len(seen)
    print(f"phase{n}: {len(ids)} ids, {len(seen)} with outcome, {dict(verdicts)}, missing {missing}, twice {twice}, extra {extra}, against expectation {wrong}, timeout {timeouts}")
print("total outcomes", total)
