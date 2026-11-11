"""Compares the fault lists of Phases 1 to 7 at two commits: which faults
were added, removed or changed (path, pairs, expectation, tests).
Usage: old_faults_diff.py <repo> <old commit> <new commit>"""
import importlib.util
import os
import subprocess
import sys
import tempfile

repo, old, new = sys.argv[1:4]
LISTS = ["PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6", "PHASE7", "PHASE7_WINDOWS"]


def load(commit):
    text = subprocess.run(["git", "-C", repo, "show", f"{commit}:mutation/faults.py"],
                          check=True, capture_output=True).stdout
    d = tempfile.mkdtemp()
    path = os.path.join(d, "faults.py")
    open(path, "wb").write(text)
    spec = importlib.util.spec_from_file_location(f"faults_{commit}", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


a, b = load(old), load(new)
for name in LISTS:
    la = {f["id"]: f for f in getattr(a, name, [])}
    lb = {f["id"]: f for f in getattr(b, name, [])}
    added = sorted(set(lb) - set(la))
    removed = sorted(set(la) - set(lb))
    changed = []
    for fid in la:
        if fid in lb:
            fa, fb = la[fid], lb[fid]
            keys = [k for k in sorted(set(fa) | set(fb)) if fa.get(k) != fb.get(k)]
            if keys:
                changed.append((fid, keys))
    print(f"{name}: {len(la)} -> {len(lb)}; added {added}; removed {removed}; changed {len(changed)}")
    for fid, keys in changed:
        fa, fb = la[fid], lb[fid]
        detail = []
        for k in keys:
            if k == "pairs":
                same_effect = [
                    (po.replace(" ", "") != pn.replace(" ", "")) for (po, _), (pn, _) in zip(fa["pairs"], fb["pairs"])
                ]
                detail.append(f"pairs({len(fa['pairs'])}->{len(fb['pairs'])})")
            else:
                detail.append(f"{k}: {fa.get(k)!r} -> {fb.get(k)!r}")
        print(f"  {fid}: " + "; ".join(detail))
