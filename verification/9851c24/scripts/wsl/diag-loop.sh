#!/bin/bash
# Runs diag-restarts.sh on one copy up to <count> times, stopping at the
# first run that does not pass; keeps every log. Changes no source file.
# Usage: diag-loop.sh <tree> <count>
set -u
here=$(dirname "$0")
for i in $(seq 1 "$2"); do
    out=$(bash "$here/diag-restarts.sh" "$1" "$1-loop$i" 1800)
    echo "$(date +%T) run $i: $(echo "$out" | grep -E '^exit|^test result' | tr '\n' ' ')"
    echo "$out" | grep -q "^exit 0 " || { echo "$out"; exit 1; }
done
echo "all $2 runs passed"
