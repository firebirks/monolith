#!/bin/bash
# Gate C on c896861 (owner, 2026-11-10): the 96 faults of run96.txt with
# the runner of c896861, two workers, in ~/p8. Runs only once the two
# unmodified checks of c2-unmodified.py have passed and been looked at.
# Writes nothing over earlier results: the runner's results file must not
# exist yet, and its log is a new one.
# Usage: targeted-c2.sh <ids file>
set -u
commit=c896861b66b0296db4ab1598589d69a88954d877
ids=$(cat "$1")
workers=2
summary=~/final8-mutation.txt
log=~/final8-mutation-C2.log
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "gate C2: not at $commit" >> "$summary"; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "gate C2: ~/p8 has changes" >> "$summary"; exit 1; }
out=mutation/results-all-${commit:0:7}.json
[ -e "$out" ] && { echo "gate C2: $out exists" >> "$summary"; exit 1; }
[ -e "$log" ] && { echo "gate C2: $log exists" >> "$summary"; exit 1; }
[ "$(echo $ids | wc -w)" = 96 ] || { echo "gate C2: not 96 ids" >> "$summary"; exit 1; }
echo "gate C2 started $(date -Is): 96 faults, $workers workers, commit $commit" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py all "$workers" $ids > "$log" 2>&1
status=$?
echo "gate C2 exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp -p "$out" ~/results-C-${commit:0:7}.json 2>/dev/null
echo "done C2" >> "$summary"
exit $status
