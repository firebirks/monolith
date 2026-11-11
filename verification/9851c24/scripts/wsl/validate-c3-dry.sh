#!/bin/bash
# Read only: validate-c2.py on the faults of gate C3 that have ended.
here=$(dirname "$0")
P=$(cd "$here/../.." && pwd)
out=$HOME/diag-c3/dry
logs=<repo>/mutation/work/logs/all-c90d5e5-20261110-151711
python3 -B "$here/validate-c3-dry.py" ~/final8-mutation-C3.log "$logs" "$out" || exit 1
python3 -B "$here/validate-c2.py" "$out/partial.json" "$logs" "$P/results" "$out/ids.txt"
