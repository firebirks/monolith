#!/bin/bash
# Read only: the files of each copy that c90d5e5 does not track, outside
# the build directory, with their times.
NEW=c90d5e54735542764fe46855ad68cbfd342273dc
cd ~/p8 || exit 1
tracked=$(mktemp)
git ls-tree -r --name-only "$NEW" | sort > "$tracked"
for tree in tree0 tree1; do
    echo "== $tree"
    (cd mutation/work/$tree && find . -path ./target -prune -o -type f -print | sed 's|^\./||' | sort) \
        | comm -23 - "$tracked" | while read -r f; do
            echo "  $(stat -c '%y %s' "mutation/work/$tree/$f" | cut -c1-19,36-) $f"
        done
done
rm -f "$tracked"
