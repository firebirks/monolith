#!/bin/bash
# Read only: the state after Windows restarted during gate C3. The runner,
# the copies (does one still hold the fault it ran?), the results, the
# logs of the two faults that did not end.
NEW=c90d5e54735542764fe46855ad68cbfd342273dc
cd ~/p8 || exit 1
echo "== processes"
pgrep -af 'mutation/run.py|targeted-c3|cargo ' | grep -v pgrep || echo "none"
echo "== ~/p8: $(git rev-parse HEAD) $(git status --porcelain --untracked-files=no | wc -l) changed"
echo "== results"
ls -l mutation/results-all-c90d5e5.json ~/results-C-c90d5e5.json 2>&1
ls -l ~/final8-mutation-C3.log; tail -2 ~/final8-mutation-C3.log
echo "== the two faults"
python3 -B - <<'PY'
import sys
sys.path.insert(0, "<repo>/mutation")
import faults
for name in ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6", "PHASE7", "PHASE8"):
    for item in getattr(faults, name):
        if item["id"] in ("FT29", "HR77"):
            print(item["id"], name, item["path"], item["crates"], item["what"])
PY
echo "== the copies against c90d5e5 (every tracked file)"
fresh=$(mktemp -d)
git archive "$NEW" | tar -x -C "$fresh"
for tree in mutation/work/tree0 mutation/work/tree1; do
    [ -d "$tree" ] || { echo "$tree: missing"; continue; }
    out=$(cd "$fresh" && git -C ~/p8 ls-tree -r --name-only "$NEW" | while read -r f; do
        cmp -s "$fresh/$f" ~/p8/"$tree/$f" || echo "    differs: $f"
    done)
    echo "$tree: ${out:-every tracked file same as c90d5e5}"
done
rm -rf "$fresh"
echo "== logs of the two faults"
ls -l --time-style=+%F_%T mutation/work/logs/all-c90d5e5-20261110-151711/{FT29,HR77}.* 2>&1
ls mutation/work/logs/all-c90d5e5-20261110-151711 | sed 's/\..*//' | sort -u | wc -l
echo "== disk"; df -h / | tail -1; free -g | head -2
