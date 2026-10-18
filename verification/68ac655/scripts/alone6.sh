#!/bin/bash
# Runs the faults given as arguments alone, one worker, on the commit of
# ~/mrun, after the mutation lists of the Phase 6 final verification of
# 68ac655, so that the tests that catch them are named. The result files of
# the lists are kept in ~/final6-results first, since run.py overwrites the
# file of the list it runs. Lines go to ~/final6-alone.txt.
set -u
trap '' HUP
phase=$1
shift
cd ~/mrun || exit 1
summary=~/final6-alone.txt
mkdir -p ~/final6-results
cp -n mutation/results-phase*-68ac655.json ~/final6-results/
echo "alone at $(git rev-parse HEAD), 1 worker, $(date -Is): $phase $*" >> "$summary"
rm -rf mutation/work
started=$SECONDS
python3 -u mutation/run.py "$phase" 1 "$@" > ~/final6-alone-"$phase".log 2>&1
code=$?
cp mutation/results-"$phase"-68ac655.json ~/final6-results/alone-"$phase"-68ac655.json
echo "$phase alone exit $code ($((SECONDS - started)) s) $(date -Is)" >> "$summary"
echo "done" >> "$summary"
