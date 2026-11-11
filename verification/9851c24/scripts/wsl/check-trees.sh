#!/bin/bash
# Compares the runner's copies in ~/p8/mutation/work with the archive of
# the commit their marker names; prints what differs.
set -u
cd ~/p8 || exit 1
fresh=$(mktemp -d)
git archive --format=tar "$(git rev-parse HEAD)" | tar -x -C "$fresh"
for tree in mutation/work/tree*; do
    echo "== $tree marker $(cat "$tree/.mutation-commit")"
    diff -rq -x target -x .mutation-commit "$fresh" "$tree" | head -20
    for f in $(diff -rq -x target -x .mutation-commit "$fresh" "$tree" | sed -n 's/^Files \([^ ]*\) and .* differ$/\1/p'); do
        rel=${f#$fresh/}
        diff -u "$f" "$tree/$rel" | head -30
    done
done
rm -rf "$fresh"
