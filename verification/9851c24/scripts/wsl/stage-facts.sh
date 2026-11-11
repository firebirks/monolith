#!/bin/bash
# Read only: the outputs of the WSL and container stages on f7f8d81 that
# the completion record cites (deny, audit, duplicates, fail-closed).
for d in ~/final8-out ~/final8-out/*; do [ -d "$d" ] && echo "dir $d"; done | head -20
cd ~/final8-out 2>/dev/null || exit 0
ls -la | head -60
for f in $(ls | grep -iE 'deny|audit|tree|fail|identity|trailer' ); do
    echo "===== $f"; tail -25 "$f" | cut -c1-200
done
