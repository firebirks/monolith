#!/bin/bash
# The mutation lists of Phases 5 to 1 of the Phase 6 final verification,
# in the WSL clone ~/mrun at the frozen commit, after the Phase 6 list ran
# on it. Waits while container.running sits next to this script, then runs
# each list with the given number of workers. A line per list goes to
# ~/final6-mutation.txt.
set -u
trap '' HUP
workers=${1:-2}
V=$(dirname "$0")
cd ~/mrun || exit 1
summary=~/final6-mutation.txt
echo "lists 5 to 1 waiting for the container stage $(date -Is)" >> "$summary"
while [ -e "$V/container.running" ]; do
    sleep 60
done
rm -rf mutation/work
echo "commit $(git rev-parse HEAD), $workers workers, lists 5 to 1, $(date -Is)" >> "$summary"
for phase in phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final6-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
