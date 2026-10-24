import json, os, re, sys
S = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
phases = ["phase1", "phase2", "phase3", "phase4", "phase5", "phase6", "phase7"]

def load(path):
    with open(path) as f:
        return json.load(f)

selected = {}
for line in open(os.path.join(S, "bbf", "affected.txt")):
    m = re.match(r"^  python3 mutation/run.py (\S+) <workers> (.*)$", line.rstrip())
    if m:
        selected[m.group(1)] = m.group(2).split()
print("selected:", {p: len(v) for p, v in selected.items()}, sum(len(v) for v in selected.values()))

bbf, old, ba = {}, {}, {}
for p in phases:
    d = load(os.path.join(S, "bbf", f"results-{p}-bbf1279.json"))
    print(p, "commit", d["commit"][:7], "unmodified", d["unmodified_seconds"], "timeout", d["timeout"], "results", len(d["results"]))
    ids = [r["id"] for r in d["results"]]
    if sorted(ids) != sorted(selected.get(p, [])) or len(ids) != len(set(ids)):
        print("  MISMATCH", sorted(ids), sorted(selected.get(p, [])))
    for r in d["results"]:
        bbf[(p, r["id"])] = r
    for r in load(os.path.join(S, "results740", f"results-{p}-740b671.json"))["results"]:
        old[(p, r["id"])] = r
    for r in load(os.path.join(S, "targeted", f"results-{p}-ba12472.json"))["results"]:
        ba[(p, r["id"])] = r

verdicts = {}
for k, r in bbf.items():
    verdicts.setdefault(r["verdict"], []).append(k[1])
print("verdicts:", {v: len(x) for v, x in verdicts.items()})
print("survived:", sorted(verdicts.get("survived", [])))

store_tests = set()
for line in open(os.path.join(S, "store-fns.txt")):
    store_tests.add(line.strip())

print("\nper fault: id | bbf verdict | 740 verdict | ba verdict | bbf failed (store.rs tests marked *)")
changes = []
for k in sorted(bbf, key=lambda k: (k[0], k[1])):
    r = bbf[k]
    o = old.get(k, {})
    b = ba.get(k, {})
    failed = r["failed"]
    marked = [("*" + f if f.split("::")[-1] in store_tests else f) for f in failed]
    flag = ""
    if r["verdict"] != o.get("verdict"):
        flag = "  <-- differs from 740b671"
        changes.append(k)
    print(f"{k[0]:7} {k[1]:6} {r['verdict']:8} {o.get('verdict','-'):8} {b.get('verdict','-'):8} {r['seconds']:5}s {' '.join(marked)[:200]}{flag}")

print("\nchanged vs 740b671:", changes)
atomic = [(k, [f for f in r["failed"] if f.split("::")[-1] in store_tests]) for k, r in bbf.items()]
atomic = [(k, f) for k, f in atomic if f]
print("\nfaults with a failing store.rs test:")
for k, f in atomic:
    others = [x for x in bbf[k]["failed"] if x.split("::")[-1] not in store_tests]
    print(" ", k, f, "| other failing:", others[:6], "| 740:", [x for x in old.get(k, {}).get("failed", []) if x.split("::")[-1] in store_tests])
