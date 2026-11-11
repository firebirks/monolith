#!/bin/bash
# Read only: the remote of ~/p8, the results files kept, the summary of
# the gates, and the disk.
cd ~/p8 || exit 1
git remote -v
git log --oneline -1
ls -la ~/p8/mutation/results-*.json ~/results-*.json 2>&1
tail -8 ~/final8-mutation.txt
ls -la ~/final8-mutation-*.log | tail -6
df -h / | tail -1
ls ~/p8/mutation/work/
du -sh ~/p8/mutation/work/tree0 ~/p8/mutation/work/tree1
