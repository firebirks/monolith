#!/bin/bash
# Read only, but for the lists it writes in ~/diag-gatec: what gate C
# built in each copy (build outputs with a time inside gate C), kept before
# a replay builds them again; and which packages depend, also as a
# development dependency, on the crates whose last restore in gate B came
# after their build (monolith-storage, monolith-core).
set -u
out=~/diag-gatec
mkdir -p "$out"
cd ~/p8/mutation/work || exit 1
for t in tree0 tree1; do
    [ -f "$out/gatec-built-$t.txt" ] && { echo "$out/gatec-built-$t.txt exists: kept"; continue; }
    find $t/target/debug/deps -maxdepth 1 -type f ! -name '*.d' \
        -newermt '2026-11-09 13:58:00' ! -newermt '2026-11-09 14:25:00' \
        -printf '%TY-%Tm-%Td %TH:%TM:%TS %s %f\n' | sort > "$out/gatec-built-$t.txt"
    echo "$t: $(wc -l < "$out/gatec-built-$t.txt") outputs written inside gate C"
done
echo "== tree1, names built by gate C (executables and libraries)"
awk '{print $4}' "$out/gatec-built-tree1.txt" | sed -E 's/-[0-9a-f]{16}(\.(rlib|rmeta))?$/ \1/' | sort | uniq -c | awk '{printf "%s%s ", $2, ($3 ? "("$3")" : "")} END {print ""}'
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
cd tree1 || exit 1
for c in monolith-storage monolith-core; do
    echo "== packages depending on $c (normal and dev)"
    cargo tree --locked --offline -i "$c" -e normal,dev --prefix none 2>&1 | grep -E '^monolith' | sed 's/ (.*//' | sort -u | tr '\n' ' '
    echo
done
echo "== tree1 sources: mtime and sha256 against the commit"
for f in crates/monolith-storage/src/local.rs crates/monolith-core/src/runtime.rs; do
    want=$(git -C ~/p8 show 9851c24b9814092c2a50efdc6da51d99e0633460:"$f" | sha256sum | cut -c1-64)
    have=$(sha256sum "$f" | cut -c1-64)
    echo "$f $(stat -c %y "$f" | cut -c1-23) same as the commit: $([ "$want" = "$have" ] && echo yes || echo NO)"
done
