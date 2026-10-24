#!/bin/bash
# The mutation part of the targeted re-verification, in the WSL clone ~/p7
# moved to the candidate: affected.py selects, against the results of the
# full run on 740b671, the faults the change can alter, and each list runs
# them. A line per list goes to ~/targeted-mutation.txt.
# Usage: targeted-mutation.sh <commit> <workers>
set -u
commit=$1
workers=$2
summary=~/targeted-mutation.txt
cd ~/p7 || exit 1
git fetch -q <winhome>/VSCodeProjects/Monolith phase-7-security-hardening || exit 1
git checkout -q --detach "$commit" || exit 1
echo "commit $(git rev-parse HEAD), $workers workers, $(date -Is)" > "$summary"
rm -rf mutation/work
mkdir -p ~/targeted-out
python3 mutation/affected.py "$commit" mutation/results-phase{1,2,3,4,5,6,7}-740b671.json > ~/targeted-out/affected.txt 2>&1 || { echo "affected.py failed" >> "$summary"; exit 1; }
grep -E '^phase' ~/targeted-out/affected.txt >> "$summary"
for phase in phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    ids=$(grep -E "^  python3 mutation/run.py $phase " ~/targeted-out/affected.txt | sed -E 's/^.*<workers> //')
    [ -n "$ids" ] || continue
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" $ids > ~/targeted-mutation-"$phase".log 2>&1
    echo "$phase exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
