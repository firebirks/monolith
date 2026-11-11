#!/bin/bash
# Read only: what gate C built in each copy, judged by the times of the
# build outputs (the six runs after it built nothing), and which sources of
# each copy were written after the copy was made (restored after a fault).
set -u
cd ~/p8/mutation/work || exit 1
echo "== summary"; tail -8 ~/final8-mutation.txt
echo "== gate C log: size $(wc -c < ~/final8-mutation-C.log) bytes; lines naming targets or results"
grep -nE "error:|target failed|targets failed|test result|FAILED|panicked|does not" ~/final8-mutation-C.log | cut -c1-220
for t in tree0 tree1; do
    echo "== $t marker $(stat -c %y $t/.mutation-commit | cut -c1-19)"
    echo "-- build outputs in deps by minute (2026-11-09), executables and rlibs"
    find $t/target/debug/deps -maxdepth 1 -type f \( -name '*.rlib' -o -perm -u+x \) ! -name '*.d' ! -name '*.so' \
        -newermt '2026-11-09 00:00' -printf '%TY-%Tm-%Td %TH:%TM %f\n' | sort | awk '{print $1, $2}' | uniq -c
    echo "-- newest 12 and oldest 5 of those"
    find $t/target/debug/deps -maxdepth 1 -type f \( -name '*.rlib' -o -perm -u+x \) ! -name '*.d' \
        -printf '%TY-%Tm-%Td %TH:%TM:%TS %s %f\n' | sort | cut -c1-140 > /tmp/ev.$t
    tail -12 /tmp/ev.$t; echo ...; head -5 /tmp/ev.$t
    echo "-- monolith rlibs"
    grep -E 'libmonolith' /tmp/ev.$t | cut -c1-120
    echo "-- sources written after the copy was made (newest 15)"
    find $t/crates -type f -newer $t/.mutation-commit -printf '%TY-%Tm-%Td %TH:%TM:%TS %p\n' | sort | tail -15 | cut -c1-140
done
