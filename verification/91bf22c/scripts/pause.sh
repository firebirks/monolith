#!/bin/bash
# Pauses the mutation lists of the final verification of 91bf22c: stops the
# loop over the lists and the runner, keeps the log of the faults that
# ended, and writes the ids still to run to ~/final-pause/remaining.txt.
# The copies are left as the stop leaves them; resume.sh makes them again.
set -u
dest=~/final-pause
mkdir -p "$dest"
for pid in $(pgrep -f "verify/(final-mutation|resume)\.sh"); do
    echo "stopping the list loop $pid"
    kill "$pid"
done
for pid in $(pgrep -f "mutation/run\.py phase"); do
    echo "stopping runner $pid: $(ps -o args= -p "$pid")"
    kill -TERM "$pid"
done
for _ in $(seq 1 30); do
    pgrep -f "mutation/run\.py|cargo test|target/debug/deps/" > /dev/null || break
    sleep 1
done
echo "left running: $(pgrep -af "mutation/run\.py|cargo test|target/debug/deps/" | grep -v pgrep | wc -l)"
rm -f ~/mrun/mutation/work/lock
cp ~/final-mutation.txt "$dest/"
for f in ~/final-mutation-phase*.log; do
    [ -e "$f" ] && cp "$f" "$dest/"
done
phase=$(ls -t ~/final-mutation-phase*.log | head -1 | sed -E 's/.*final-mutation-(phase[0-9])\.log/\1/')
echo "$phase" > "$dest/phase.txt"
cd ~/mrun || exit 1
python3 - "$phase" ~/final-mutation-"$phase".log "$dest/remaining.txt" <<'PY'
import re, sys
sys.path.insert(0, "mutation")
import faults
phase, log, out = sys.argv[1:]
items = {"phase1": faults.PHASE1, "phase2": faults.PHASE2, "phase3": faults.PHASE3,
         "phase4": faults.PHASE4, "phase5": faults.PHASE5}[phase]
ended = set(re.findall(r"^([A-Z]+[0-9]+) +(?:caught|survived|invalid) ", open(log).read(), re.M))
remaining = [item["id"] for item in items if item["id"] not in ended]
open(out, "w").write(" ".join(remaining) + "\n")
print(f"{phase}: {len(ended)} ended, {len(remaining)} to run: {' '.join(remaining)}")
PY
date -Is > "$dest/paused-at.txt"
