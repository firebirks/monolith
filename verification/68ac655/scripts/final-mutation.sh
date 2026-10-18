#!/bin/bash
# The mutation lists of the Phase 6 final verification, in the WSL clone
# ~/mrun at the frozen commit: Phase 6 first, then Phases 5 to 1, each with
# the given number of workers. A line per phase goes to
# ~/final6-mutation.txt. While the file container.running sits next to
# this script, the container stage holds the memory of the VM, and the
# lists wait for it to end.
set -u
workers=${1:-2}
V=$(dirname "$0")
cd ~/mrun || exit 1
summary=~/final6-mutation.txt
echo "waiting for the container stage $(date -Is)" > "$summary"
while [ -e "$V/container.running" ]; do
    sleep 60
done
echo "commit $(git rev-parse HEAD), $workers workers, $(date -Is)" >> "$summary"
for phase in phase6 phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final6-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
