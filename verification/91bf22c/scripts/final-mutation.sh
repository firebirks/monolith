#!/bin/bash
# The mutation lists of the final verification, in the WSL clone ~/mrun at
# the frozen commit: Phase 5 first, then Phases 4 to 1, each with the
# given number of workers. A line per phase goes to ~/final-mutation.txt.
set -u
workers=${1:-2}
cd ~/mrun || exit 1
summary=~/final-mutation.txt
echo "commit $(git rev-parse HEAD), $workers workers, $(date -Is)" > "$summary"
for phase in phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
