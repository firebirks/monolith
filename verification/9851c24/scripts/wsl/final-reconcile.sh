#!/bin/bash
# Read only: final-reconcile.py, and the diffs that bound the evidence of
# the Windows lists (f7f8d81..9851c24) and of the candidate (9851c24..c90d5e5).
here=$(dirname "$0")
P=$(cd "$here/../.." && pwd)
ls -l ~/final8-mutation-phase7-part1.log ~/final8-mutation-phase8.log
python3 -B "$here/final-reconcile.py" "$P"
cd ~/p8 || exit 1
echo; echo "== f7f8d81..9851c24 (the Windows lists ran on f7f8d81)"
git diff --stat f7f8d81 9851c24 | tail -8
echo; echo "== 9851c24..c90d5e5"
git diff --stat 9851c24 c90d5e5 | tail -8
