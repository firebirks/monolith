#!/bin/bash
# Gate C3 after Windows restarted (2026-11-11): the two of the 98 that did
# not end, FT29 and HR77, with the runner of c896861, two workers, in
# ~/p8, once the unmodified checks of c3r-unmodified.py passed. The 96
# that ended keep their log (~/final8-mutation-C3.log) and their logs
# directory; this writes over neither. The runner's log also goes to
# gate-c3r.log beside the launcher.
set -u
here=$(dirname "$0")
commit=c90d5e54735542764fe46855ad68cbfd342273dc
ids="FT29 HR77"
workers=2
summary=~/final8-mutation.txt
log=~/final8-mutation-C3r.log
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "gate C3r: not at $commit" >> "$summary"; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "gate C3r: ~/p8 has changes" >> "$summary"; exit 1; }
out=mutation/results-all-${commit:0:7}.json
[ -e "$out" ] && { echo "gate C3r: $out exists" >> "$summary"; exit 1; }
[ -e "$log" ] && { echo "gate C3r: $log exists" >> "$summary"; exit 1; }
echo "gate C3r started $(date -Is): $ids, $workers workers, commit $commit" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py all "$workers" $ids 2>&1 | tee "$log" > "$here/../gate-c3r.log"
status=${PIPESTATUS[0]}
echo "gate C3r exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp -p "$out" ~/results-C-${commit:0:7}-FT29-HR77.json 2>/dev/null
echo "done C3r" >> "$summary"
exit $status
