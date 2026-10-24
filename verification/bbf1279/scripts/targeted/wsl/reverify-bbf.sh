#!/bin/bash
# The WSL part of the targeted re-verification of a candidate after the
# test-only fix of the store crash tests: test B again (4 copies of the
# store binary at once, 3 rounds), then the mutation faults affected.py
# selects against the results of 740b671 overlaid with those of ba12472.
# Usage: reverify-bbf.sh <commit> <workers>
set -u
commit=$1
workers=$2
summary=~/reverify-bbf.txt
cd ~/p7 || exit 1
git fetch -q <winhome>/VSCodeProjects/Monolith phase-7-security-hardening || { echo "fetch failed" > "$summary"; exit 1; }
git checkout -q --detach "$commit" || { echo "checkout failed" > "$summary"; exit 1; }
echo "commit $(git rev-parse HEAD), changed files $(git status --porcelain | grep -v '^??' | wc -l), $workers workers, $(date -Is)" > "$summary"
rm -rf mutation/work

# B again, in a target directory of its own.
out=~/reverify-bbf-B
rm -rf "$out"
mkdir -p "$out"
B=$(CARGO_TARGET_DIR=~/bbf-target cargo test -p monolith-core --test store --no-run --message-format=json 2>"$out/build.log" | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "store":
        print(m["executable"])')
[ -x "$B" ] || { echo "B: no binary" >> "$summary"; exit 1; }
echo "B: binary $B sha256 $(sha256sum "$B" | cut -d' ' -f1)" >> "$summary"
(
    cd crates/monolith-core || exit 1
    export CARGO_MANIFEST_DIR=$PWD
    for round in 1 2 3; do
        for c in 1 2 3 4; do
            ( s=$SECONDS; "$B" > "$out/B$round-$c.log" 2>&1; echo "rc=$? $((SECONDS - s))s" > "$out/B$round-$c.rc" ) &
        done
        wait
        for c in 1 2 3 4; do
            echo "B$round copy $c $(cat "$out/B$round-$c.rc") $(grep -E '^test result' "$out/B$round-$c.log") failed: $(grep -E '^test .* FAILED$' "$out/B$round-$c.log" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | tr '\n' ' ')" >> "$summary"
        done
    done
)

# The mutation faults.
mkdir -p ~/reverify-bbf-out
python3 mutation/affected.py "$commit" mutation/results-phase{1,2,3,4,5,6,7,7w}-740b671.json mutation/results-phase{1,2,3,4,5,6,7}-ba12472.json > ~/reverify-bbf-out/affected.txt 2>&1 || { echo "affected.py failed" >> "$summary"; exit 1; }
grep -E '^phase' ~/reverify-bbf-out/affected.txt >> "$summary"
for phase in phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    ids=$(grep -E "^  python3 mutation/run.py $phase " ~/reverify-bbf-out/affected.txt | sed -E 's/^.*<workers> //')
    [ -n "$ids" ] || continue
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" $ids > ~/reverify-bbf-mutation-"$phase".log 2>&1
    echo "$phase exit $? ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
done
echo "done" >> "$summary"
