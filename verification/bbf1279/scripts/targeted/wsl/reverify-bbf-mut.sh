#!/bin/bash
# The mutation part of the targeted re-verification, resumed: the phases
# whose results are not yet in ~/reverify-bbf-out, then, again, the faults
# that were running while the watchdog had the run stopped (mutpause.sh),
# until none is left or three rounds have run. Every log line starts with
# the time it was written, in seconds (paused-faults.py reads it).
# Usage: reverify-bbf-mut.sh <commit> <workers>
set -u
commit=$1
workers=$2
short=${commit:0:7}
summary=~/reverify-bbf-mut.txt
out=~/reverify-bbf-out
cd ~/p7 || exit 1
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "HEAD is not $commit" >> "$summary"; exit 1; }
echo "commit $(git rev-parse HEAD), changed files $(git status --porcelain | grep -v '^??' | wc -l), $workers workers, $(date -Is)" >> "$summary"
rm -rf mutation/work

wait_unpaused() {
    while [ -e ~/reverify-bbf.paused ]; do sleep 30; done
}

# run_phase <phase> <label> <ids...>
run_phase() {
    local phase=$1 label=$2
    shift 2
    wait_unpaused
    rm -f "mutation/results-$phase-$short.json"
    local started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" "$@" < /dev/null 2>&1 |
        while IFS= read -r line; do printf '%s %s\n' "$(date +%s)" "$line"; done > ~/reverify-bbf-mutation-"$label".log
    local rc=${PIPESTATUS[0]}
    [ -f "mutation/results-$phase-$short.json" ] && cp "mutation/results-$phase-$short.json" "$out/results-$label-$short.json"
    echo "$label exit $rc ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
}

for phase in phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    [ -f "$out/results-$phase-$short.json" ] && continue
    ids=$(grep -E "^  python3 mutation/run.py $phase " "$out/affected.txt" | sed -E 's/^.*<workers> //')
    [ -n "$ids" ] || continue
    run_phase "$phase" "$phase" $ids
done

logs=$(ls ~/reverify-bbf-mutation-phase?.log)
for round in 1 2 3; do
    wait_unpaused
    python3 ~/paused-faults.py $logs > "$out/paused-$round.txt" || { echo "paused-faults.py failed" >> "$summary"; break; }
    [ -s "$out/paused-$round.txt" ] || break
    echo "round $round, again: $(tr '\n' ';' < "$out/paused-$round.txt")" >> "$summary"
    while read -r -u 3 phase ids; do
        run_phase "$phase" "$phase-rerun$round" $ids
    done 3< "$out/paused-$round.txt"
    logs=$(ls ~/reverify-bbf-mutation-*-rerun$round.log)
done
echo "done" >> "$summary"
