#!/bin/bash
# Waits until `file` has more than `seen` lines matching the runner's
# anomaly messages (a fault not caught, a timeout verdict, a copy not
# restored, a precheck or unmodified-run failure, a Python error), or a
# line matching `end`; then prints the matching lines.
# Usage: wait-anomaly.sh <file> <seen> <end>
file=$1 seen=$2 end=$3
pattern='does not end its tests|does not pass its tests|was not restored|pattern found|\(timeout\)$| survived | invalid |Traceback|^done:'
until [ "$(grep -cE "$pattern" "$file" 2>/dev/null)" -gt "$seen" ] || grep -qE "$end" "$file" 2>/dev/null; do
    sleep 60
done
grep -nE "$pattern" "$file" | cut -c1-200
