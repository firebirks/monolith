#!/bin/bash
# Gate B of the Phase 8 convergence plan (owner approval 2026-11-07, go
# 2026-11-08): the 84 Phase 1-7 faults selected by touched_functions.py
# (78, mutated text in a function edited since b93a10a) and
# touched_faults.py (6, caught last only by tests in changed files), in one
# run of the official runner in ~/p8 at the commit given.
# Usage: targeted-mutation.sh <commit> [workers]
set -u
commit=$1
workers=${2:-2}
summary=~/final8-mutation.txt
ids="X8 X9 X10 B1 B2 B5 CS46 CS100 MR8 MR9 MR23 MR26 MR27 MR28 MR30 MR32 MR33 MR34 MR35 MR38 MR39 MR40 MR41 MR44 MR53 MR54 MR57 UI1 UI2 UI4 UI5 UI9 UI13 UI15 UI18 UI21 UI25 UI26 UI27 UI44 UI47 UI52 UI53 UI54 UI55 UI58 UI71 UI73 UI74 UI81 UI82 UI83 UI84 UI92 UI93 UI109 UI110 UI116 UI118 UI141 HR29 HR30 HR31 HR32 HR33 HR35 HR36 HR37 HR38 HR39 HR40 HR53 HR60 HR63 HR65 HR68 HR76 HR85 CS110 CS113 CS116 CS117 UI104 UI158"
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
git checkout -q --detach "$commit" || { echo "cannot check out $commit" >> "$summary"; exit 1; }
[ "$(git rev-parse HEAD)" = "$commit" ] || { echo "not at $commit" >> "$summary"; exit 1; }
echo "gate B started $(date -Is): $(echo $ids | wc -w) faults, $workers workers, commit $commit" >> "$summary"
started=$SECONDS
python3 -u mutation/run.py all "$workers" $ids > ~/final8-mutation-B.log 2>&1
status=$?
echo "gate B exit $status ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
cp mutation/results-all-"${commit:0:7}".json ~/results-B-"${commit:0:7}".json 2>/dev/null
echo "done B" >> "$summary"
