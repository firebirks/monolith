#!/bin/bash
# Gate C on c90d5e5 (owner, 2026-11-10): the 98 faults of run98.txt with
# the runner of c896861, two workers, in ~/p8, once the two unmodified
# checks of c3-unmodified.py passed. Writes nothing over earlier results.
# The runner's log also goes to gate-c3.log beside the launcher.
# Usage: targeted-c3.sh <ids file>
set -u
here=$(dirname "$0")
commit=c90d5e54735542764fe46855ad68cbfd342273dc
ids=$(cat "$1")
workers=2
summary=~/final8-mutation.txt
log=~/final8-mutation-C3.log
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "gate C3: not at $commit" >> "$summary"; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "gate C3: ~/p8 has changes" >> "$summary"; exit 1; }
out=mutation/results-all-${commit:0:7}.json
[ -e "$out" ] && { echo "gate C3: $out exists" >> "$summary"; exit 1; }
[ -e "$log" ] && { echo "gate C3: $log exists" >> "$summary"; exit 1; }
[ "$(echo $ids | wc -w)" = 98 ] || { echo "gate C3: not 98 ids" >> "$summary"; exit 1; }
echo "gate C3 started $(date -Is): 98 faults, $workers workers, commit $commit" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py all "$workers" $ids 2>&1 | tee "$log" > "$here/../gate-c3.log"
status=${PIPESTATUS[0]}
echo "gate C3 exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp -p "$out" ~/results-C-${commit:0:7}.json 2>/dev/null
echo "done C3" >> "$summary"
exit $status
