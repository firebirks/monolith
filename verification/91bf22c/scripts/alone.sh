#!/bin/bash
# After the lists of the final verification of 91bf22c: runs alone, with
# one worker, the faults whose catch on 91bf22c came from the timeout in a
# segment without a JSON (MR9, Q1, T6), so that the tests they make fail
# are recorded. The JSONs of the lists are kept in ~/final-results first,
# since the runner writes results-<phase>-<commit>.json whatever it runs.
set -u
cd ~/mrun || exit 1
out=~/final-alone.txt
SP=<work>
grep -q '^done' ~/final-mutation.txt || { echo "the lists have not ended" >> "$out"; exit 1; }
mkdir -p ~/final-results
cp -n mutation/results-*-91bf22c.json ~/final-results/
# The copies are made again from the commit, as resume.sh does.
rm -rf mutation/work/tree*
echo "start at $(git rev-parse HEAD), $(date -Is)" >> "$out"
for spec in "phase5 MR9" "phase3 Q1" "phase2 T6"; do
    set -- $spec
    phase=$1 id=$2
    started=$SECONDS
    python3 -u mutation/run.py "$phase" 1 "$id" > ~/final-alone-"$id".log 2>&1
    code=$?
    mv mutation/results-"$phase"-91bf22c.json ~/final-results/results-"$phase"-91bf22c-"$id".json
    cp ~/final-results/results-"$phase"-91bf22c.json mutation/
    echo "$id exit $code ($((SECONDS - started)) s) $(date -Is)" >> "$out"
    grep -E "^commit |^$id " ~/final-alone-"$id".log >> "$out"
    python3 "$SP/verify/timeout-tests.py" "$id" ~/final-results/results-"$phase"-91bf22c-"$id".json >> "$out"
done
echo "done" >> "$out"
