#!/bin/bash
# The mutation lists of the Phase 7 final verification, in the WSL clone
# ~/p7 at the frozen commit: Phase 7 first, then Phases 6 to 1, each with
# the given number of workers. A line per list goes to
# ~/final7-mutation.txt. The Windows list (phase7w) runs on Windows.
# Usage: final-mutation.sh <commit> [workers]
set -u
commit=$1
workers=${2:-2}
cd ~/p7 || exit 1
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" > ~/final7-mutation.txt; exit 1; }
summary=~/final7-mutation.txt
echo "commit $(git rev-parse HEAD), $workers workers, $(date -Is)" > "$summary"
for phase in phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final7-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
