#!/bin/bash
# The complete Phase 1 to 8 mutation baseline of the Phase 8 final
# verification, in the WSL clone ~/p8 at the frozen commit: Phase 8 first,
# then Phases 7 to 1, each with the given number of workers. A line per
# list goes to ~/final8-mutation.txt. The Windows lists (phase7w, phase8w)
# run on Windows.
#
# It waits for the file mutation-go next to this script, which names the
# commit to run: the candidate given to the chain was replaced, and the
# new one is checked on its own before the baseline starts, with nothing
# heavy beside it.
# Usage: final-mutation.sh <commit> [workers]
set -u
here=$(dirname "$0")
workers=${2:-2}
summary=~/final8-mutation.txt
echo "waiting for $here/mutation-go since $(date -Is) (chain gave $1)" > "$summary"
until [ -s "$here/mutation-go" ]; do sleep 60; done
commit=$(tr -d '\r\n ' < "$here/mutation-go")
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
git -c safe.directory='*' fetch -q <repo> phase-8-file-transfer
git checkout -q --detach "$commit" || { echo "cannot check out $commit" >> "$summary"; exit 1; }
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" >> "$summary"; exit 1; }
echo "commit $(git rev-parse HEAD), $workers workers, $(date -Is)" > "$summary"
for phase in phase8 phase7 phase6 phase5 phase4 phase3 phase2 phase1; do
    started=$SECONDS
    python3 -u mutation/run.py "$phase" "$workers" > ~/final8-mutation-"$phase".log 2>&1
    status=$?
    echo "$phase exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
    cp mutation/results-"$phase"-"${commit:0:7}".json ~/ 2>/dev/null
done
echo "done" >> "$summary"
