#!/bin/bash
# Read only on the evidence: the validation of gate C3 (98 on c90d5e5).
# Writes into ~/diag-c3/final only.
here=$(dirname "$0")
P=$(cd "$here/../.." && pwd)
out=~/diag-c3/final
[ -e "$out" ] && { echo "$out exists"; exit 2; }
python3 -B "$here/c3-merge.py" "$out" || { echo "MERGE PROBLEMS"; }
echo
python3 -B "$here/validate-c2.py" "$out/results-C3-merged.json" "$out/logs" "$P/results" "$P/run98.txt"
echo
python3 -B "$here/c3-extra.py" "$out"
