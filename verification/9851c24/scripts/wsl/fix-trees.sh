#!/bin/bash
# Brings the runner's copies back to the commit their marker names: each
# file that differs is written again from the archive with a new mtime, so
# that cargo builds it again; each file the commit does not have is
# removed. Build outputs (target) are kept.
set -u
cd ~/p8 || exit 1
fresh=$(mktemp -d)
git archive --format=tar "$(git rev-parse HEAD)" | tar -x -C "$fresh"
for tree in mutation/work/tree*; do
    [ "$(cat "$tree/.mutation-commit")" = "$(git rev-parse HEAD)" ] || { echo "$tree: other commit"; continue; }
    diff -rq -x target -x .mutation-commit "$fresh" "$tree" | while read -r line; do
        case "$line" in
            "Files "*)
                src=$(echo "$line" | sed -n 's/^Files \([^ ]*\) and .* differ$/\1/p')
                rel=${src#$fresh/}
                cat "$src" > "$tree/$rel"
                touch "$tree/$rel"
                echo "$tree: restored $rel"
                ;;
            "Only in $tree"*)
                dir=$(echo "$line" | sed -n 's/^Only in \([^:]*\): \(.*\)$/\1/p')
                name=$(echo "$line" | sed -n 's/^Only in \([^:]*\): \(.*\)$/\2/p')
                rm -rf "$dir/$name"
                echo "$tree: removed ${dir#$tree/}/$name"
                ;;
            *)
                echo "$tree: left as is: $line"
                ;;
        esac
    done
    echo "$tree: differences now: $(diff -rq -x target -x .mutation-commit "$fresh" "$tree" | wc -l)"
done
rm -rf "$fresh"
