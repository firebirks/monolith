#!/bin/bash
# Waits until the final mutation run finishes one more list than when it
# was started (or finishes altogether), then prints the summary and what
# did not come out as expected in the lists finished so far.
summary=~/final6-mutation.txt
before=$(grep -c " exit " "$summary")
while [ "$(grep -c " exit " "$summary")" -le "$before" ] && ! grep -q "^done" "$summary"; do
    sleep 300
done
cat "$summary"
for log in ~/final6-mutation-phase*.log; do
    echo "== $log"
    grep -E "^done|unmodified run" "$log"
    grep -E "^[A-Z]+[0-9]+ +(survived|invalid|timeout)" "$log"
done
