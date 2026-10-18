#!/bin/bash
# Runs the faults named as arguments of the Phase 6 mutation list (all of
# it without arguments) on the head of the local branch
# phase-6-desktop-client, in the WSL clone ~/mrun. Started by
# launch-wsl.ps1 from a detached Windows process, so that WSL stays up
# while it runs; a hang-up does not stop it.
set -u
trap '' HUP
log=~/p6-faults-run.log
cd ~/mrun || exit 1
rm -rf mutation/work
git fetch -q <winhome>/VSCodeProjects/Monolith phase-6-desktop-client || { echo "fetch failed" > "$log"; exit 1; }
git checkout -q -B phase-6 FETCH_HEAD
echo "head $(git rev-parse --short HEAD), faults: ${*:-all of phase6}, $(date -Is)" > "$log"
python3 -u mutation/run.py phase6 2 "$@" >> "$log" 2>&1
echo "exit $? $(date -Is)" >> "$log"
