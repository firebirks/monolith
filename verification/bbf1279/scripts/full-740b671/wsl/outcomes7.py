# Checks that every fault id of each list has exactly one outcome line in
# the logs of the Phase 7 final mutation run on 740b671, and lists timeout
# catches. The phase4 log stopped when the VM stopped keeps the faults that
# concluded; the resumed log has the others.
import collections, glob, os, re, sys
sys.path.insert(0, os.path.expanduser("~/p7/mutation"))
import faults
home = os.path.expanduser("~")
total = 0
for n in range(7, 0, -1):
    ids = [f["id"] for f in getattr(faults, f"PHASE{n}")]
    expect = {f["id"]: f["expect"] for f in getattr(faults, f"PHASE{n}")}
    seen = collections.defaultdict(list)
    for log in sorted(glob.glob(f"{home}/final7-mutation-phase{n}*.log")):
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
    survived = sorted(i for i, v in seen.items() if v[0][0] == "survived")
    total += len(seen)
    print(f"phase{n}: {len(ids)} ids, {len(seen)} with outcome, {dict(verdicts)}, missing {missing}, twice {twice}, extra {extra}, against expectation {wrong}, survived {survived}, timeout {timeouts}")
print("total outcomes", total)
