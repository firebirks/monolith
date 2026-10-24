#!/bin/bash
# Resumes the mutation lists of the Phase 7 final verification after the
# WSL VM stopped during phase4: the faults of phase4 that had concluded are
# kept, the others run again from the start, then phases 3 to 1. The work
# copies are made again from the commit, since a stopped run can leave a
# fault applied in one of them.
# Usage: resume-mutation.sh <commit> <workers>
set -u
commit=$1
workers=$2
cd ~/p7 || exit 1
summary=~/final7-mutation.txt
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" >> "$summary"; exit 1; }
rm -rf mutation/work
mv ~/final7-mutation-phase4.log ~/final7-mutation-phase4-interrupted.log
done_ids=$(grep -oE '^[A-Z]+[0-9]+ +(caught|survived)' ~/final7-mutation-phase4-interrupted.log | awk '{print $1}')
left=$(python3 - "$done_ids" <<'P'
import sys
sys.path.insert(0, "mutation")
import faults
done = set(sys.argv[1].split())
print(" ".join(f["id"] for f in faults.PHASE4 if f["id"] not in done))
P
)
echo "resumed $(date -Is): phase4 kept $(echo $done_ids | wc -w), runs again $(echo $left | wc -w), $workers workers" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py phase4 "$workers" $left > ~/final7-mutation-phase4-resumed.log 2>&1
echo "phase4 resumed exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
for phase in phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final7-mutation-"$phase".log 2>&1
    echo "$phase exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
