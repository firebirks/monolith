#!/bin/bash
# Gate C: the 96 Phase 1-7 faults of the corrected selection (owner,
# 2026-11-09): the 92 selected and not run, and MR25, K4, CS30 and Q1,
# caught before by a timeout or with no test named. In WSL on the commit
# given. The results of gate B have the same name: they are moved aside
# first, and only if they equal the copy kept in the home directory.
# Usage: targeted-c.sh <commit> <ids file> [workers]
set -u
commit=$1
ids=$(cat "$2")
workers=${3:-2}
summary=~/final8-mutation.txt
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
git checkout -q --detach "$commit" || { echo "cannot check out $commit" >> "$summary"; exit 1; }
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" >> "$summary"; exit 1; }
[ -z "$(git status --porcelain --untracked-files=no)" ] || { echo "~/p8 has changes" >> "$summary"; exit 1; }
out=mutation/results-all-${commit:0:7}.json
if [ -f "$out" ]; then
    cmp -s "$out" ~/results-B-"${commit:0:7}".json || { echo "$out differs from the gate B copy" >> "$summary"; exit 1; }
    mv "$out" mutation/results-all-"${commit:0:7}"-B.json
fi
echo "gate C started $(date -Is): $(echo $ids | wc -w) faults, $workers workers, commit $commit" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py all "$workers" $ids > ~/final8-mutation-C.log 2>&1
status=$?
echo "gate C exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp "$out" ~/results-C-"${commit:0:7}".json 2>/dev/null
echo "done C" >> "$summary"
