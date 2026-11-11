#!/bin/bash
# Read-only evidence of gate B in ~/p8: commit, clean checkout, the
# runner and the fault list against the commit's blobs, the results file.
set -u
cd ~/p8 || exit 1
echo "HEAD $(git rev-parse HEAD)"
echo "status lines: $(git status --porcelain --untracked-files=no | wc -l)"
for f in mutation/run.py mutation/faults.py; do
    echo "$f blob $(git rev-parse HEAD:$f) file $(git hash-object $f) sha256 $(sha256sum $f | cut -c1-64)"
done
sha256sum mutation/results-all-9851c24.json ~/results-B-9851c24.json
python3 - <<'EOF'
import json
d = json.load(open("mutation/results-all-9851c24.json"))
r = d["results"]
ids = [x["id"] for x in r]
print("commit", d["commit"], "unmodified", d["unmodified_seconds"], "timeout", d["timeout"])
print("results", len(r), "unique ids", len(set(ids)))
print("verdicts", {v: sum(x["verdict"] == v for x in r) for v in set(x["verdict"] for x in r)})
print("expect values", sorted(set(x["expect"][:12] for x in r)))
print("seeds_only", [x["id"] for x in r if x["seeds_only"]])
print("marks", [(x["id"], [f for f in x["failed"] if f.startswith("(")]) for x in r if any(f.startswith("(") for f in x["failed"])])
print("no test named", [x["id"] for x in r if not [f for f in x["failed"] if not f.startswith("(")]])
EOF
grep -c "^gate B exit 0" ~/final8-mutation.txt
