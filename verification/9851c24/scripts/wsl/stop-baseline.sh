#!/bin/bash
# Stops what is left of the mutation baseline: the runner, if it is still
# there, and the tests it started in its copies.
ps -eo pid,ppid,pgid,etime,args | awk 'NR == 1 || /run\.py|final-mutation|cargo test|mutation\/work/' | grep -v awk | cut -c1-160
for pid in $(pgrep -f 'run\.py'); do
    echo "stopping run.py $pid"
    kill -TERM "$pid"
done
sleep 3
for pid in $(pgrep -f 'cargo test --locked --no-fail-fast'); do
    pgid=$(ps -o pgid= -p "$pid" | tr -d ' ')
    echo "stopping cargo $pid group $pgid"
    kill -TERM -- "-$pgid" 2>/dev/null || kill -TERM "$pid"
done
sleep 5
echo "left:"
pgrep -af 'run\.py|final-mutation|mutation/work' | cut -c1-160 || echo none
ls -la ~/p8/mutation/work/ 2>/dev/null | head
ls -la ~/final8-mutation-phase7.log 2>/dev/null
