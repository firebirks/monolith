#!/bin/bash
# Read only: the steps of a replay cycle, how WSL started, and when the
# build ended and transfer_restarts ran in the unmodified run of tree1.
d=~/diag-gatec/cycle$1
head -3 "$d/conditions.txt"
cat "$d/steps.txt"
for f in "$d"/*tree1-unmodified.stamped; do
    [ -f "$f" ] || continue
    grep -E " E +(Compiling|Finished)| E +Running (unittests src/lib.rs \(target/debug/deps/monolith_core|tests/transfer_restarts)" "$f" | sed -n '1p;$p;/Finished/p;/transfer_restarts/p;/monolith_core-/p' | sort -u | cut -c1-160
    grep -nE "^[0-9:.]+ O test result" "$f" | head -3 | cut -c1-160
done
