#!/bin/bash
# Resumes the mutation baseline of the Phase 8 final verification after the
# interruption of 2026-11-05 18:25 (WSL shut down for memory): the faults
# of Phase 8 that had not ended (FT1 to FT25 were caught, in
# ~/final8-mutation-phase8.log, kept as ~/final8-mutation-phase8-part1.log),
# then Phases 7 to 1, in ~/p8 at the commit given.
# Usage: resume-mutation.sh <commit> [workers]
set -u
commit=$1
workers=${2:-2}
summary=~/final8-mutation.txt
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
git checkout -q --detach "$commit" || { echo "cannot check out $commit" >> "$summary"; exit 1; }
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" >> "$summary"; exit 1; }
[ -f ~/final8-mutation-phase8-part1.log ] || cp ~/final8-mutation-phase8.log ~/final8-mutation-phase8-part1.log
done_ids=$(sed -n 's/^\(FT[0-9]*\) .*/\1/p' ~/final8-mutation-phase8-part1.log)
rest=$(python3 -c "
import sys
sys.path.insert(0, 'mutation')
import faults
done = set(sys.argv[1:])
print(' '.join(f['id'] for f in faults.PHASE8 if f['id'] not in done))
" $done_ids)
echo "resumed $(date -Is): phase8 done before: $(echo $done_ids | wc -w), to run: $(echo $rest | wc -w)" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py phase8 "$workers" $rest > ~/final8-mutation-phase8-part2.log 2>&1
status=$?
echo "phase8 (part 2) exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp mutation/results-phase8-"${commit:0:7}".json ~/results-phase8-"${commit:0:7}"-part2.json 2>/dev/null
for phase in phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final8-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
    cp mutation/results-"$phase"-"${commit:0:7}".json ~/ 2>/dev/null
done
echo "done" >> "$summary"
