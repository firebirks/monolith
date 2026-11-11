#!/bin/bash
# Read only: the validation of gate C3 (98 faults on c90d5e5), with the
# validation script of gate C. Works on partial results too (the problems
# then list the faults not run yet).
here=$(dirname "$0")
P=$(cd "$here/../.." && pwd)
results=<repo>/mutation/results-all-c90d5e5.json
[ -e "$results" ] || { echo "no $results yet"; exit 1; }
python3 "$here/validate-c2.py" "$results" \
    <repo>/mutation/work/logs/all-c90d5e5-20261110-151711 \
    "$P/results" "$P/run98.txt"
