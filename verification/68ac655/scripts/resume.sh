#!/bin/bash
# Resumes the mutation lists of the Phase 6 final verification of 68ac655
# after pause.sh: the copies are made again from the commit, the faults of
# the paused list that had not ended run, then the lists after it, as
# final-mutation-rest.sh would have run them. Lines go to
# ~/final6-mutation.txt as before.
set -u
trap '' HUP
workers=${1:-2}
cd ~/mrun || exit 1
summary=~/final6-mutation.txt
phase=$(cat ~/final6-pause/phase.txt)
remaining=$(cat ~/final6-pause/remaining.txt)
echo "resumed at $(git rev-parse HEAD), $workers workers, $(date -Is): $phase from $(echo "$remaining" | wc -w) faults" >> "$summary"
rm -rf mutation/work
started=$SECONDS
# shellcheck disable=SC2086
python3 -u mutation/run.py "$phase" "$workers" $remaining > ~/final6-mutation-"$phase"-resumed.log 2>&1
echo "$phase (resumed) exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
for next in phase5 phase4 phase3 phase2 phase1; do
    # Only the lists after the paused one.
    [ "$next" \< "$phase" ] || continue
    started=$SECONDS
    python3 -u mutation/run.py "$next" "$workers" > ~/final6-mutation-"$next".log 2>&1
    echo "$next exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
