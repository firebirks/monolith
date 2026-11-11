#!/bin/bash
# Read only, but for the list it writes: the sources of tree1 the runner
# restored during gate B (written between its start and its end), each
# with its time and whether its content is the commit's. The replay gives
# these a new time again, as the restore did, before building tree1.
set -u
out=~/diag-gatec
cd ~/p8/mutation/work/tree1 || exit 1
list=$out/touch-tree1.txt
[ -f "$list" ] && { echo "$list exists: kept"; cat "$list"; exit 0; }
find crates -type f -newermt '2026-11-08 23:12:42' ! -newermt '2026-11-09 06:01:32' -printf '%TY-%Tm-%Td %TH:%TM:%TS %p\n' | sort > /tmp/touch.$$
bad=0
while read -r day time path; do
    want=$(git -C ~/p8 show 9851c24b9814092c2a50efdc6da51d99e0633460:"$path" | sha256sum | cut -c1-64)
    have=$(sha256sum "$path" | cut -c1-64)
    [ "$want" = "$have" ] || { echo "NOT the commit's content: $path" >&2; bad=1; }
    echo "$path"
done < /tmp/touch.$$ > "$list.tmp"
cat /tmp/touch.$$ | cut -c1-120
rm -f /tmp/touch.$$
[ $bad = 0 ] || { echo "list not written"; rm -f "$list.tmp"; exit 1; }
mv "$list.tmp" "$list"
echo "$(wc -l < "$list") sources, all as in the commit; crates: $(cut -d/ -f2 "$list" | sort -u | tr '\n' ' ')"
