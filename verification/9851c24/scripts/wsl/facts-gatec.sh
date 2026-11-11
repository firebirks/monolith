#!/bin/bash
# Read only: the steps of cycles 2 and 3, where the tests' temporary
# files live, the CPU steal sampled while transfer_restarts ran, and the
# state of the copies after the replay.
here=$(dirname "$0")
for c in 2 3; do
    echo "######## cycle $c"
    grep -E "uptime|WSL started" ~/diag-gatec/cycle$c/conditions.txt | head -2
    cut -c1-700 ~/diag-gatec/cycle$c/steps.txt
done
echo "== binaries run by the tree1 runs: identical across cycles?"
md5sum ~/diag-gatec/cycle*/tree1-unmodified.binaries.json ~/diag-unmod/run1.binaries.json | cut -c1-12,34-
echo "== temporary directory: TMPDIR=${TMPDIR:-unset}"
findmnt -T /tmp -o TARGET,SOURCE,FSTYPE,OPTIONS | cut -c1-120
findmnt -T "$HOME" -o TARGET,SOURCE,FSTYPE | cut -c1-120
echo "== CPU steal and pressure while transfer_restarts ran (samples naming it)"
for c in 1 2 3; do
    grep "transfer_restarts" ~/diag-gatec/cycle$c/io.log | awk -v c=$c '{
        for (i = 1; i <= NF; i++) { if ($i == "st") st = $(i+1); if ($i == "cpu" && $(i+1) ~ /%$/ && $(i-1) ~ /%$/) p = $(i+1) }
        n++; gsub("%", "", st); if (st + 0 > m) m = st + 0; s += st }
        END { printf "cycle %s: %d samples, steal mean %.1f%% max %d%%\n", c, n, s / n, m }'
done
echo "== copies after the replay"
cd ~/p8/mutation/work
for t in tree0 tree1; do
    echo "$t: marker $(cat $t/.mutation-commit | cut -c1-7); regression files: $(find $t/crates -name '*.proptest-regressions' | wc -l)"
done
cd ~/p8 && echo "~/p8 HEAD $(git rev-parse --short HEAD), status: $(git status --porcelain --untracked-files=no | wc -l) changed"
