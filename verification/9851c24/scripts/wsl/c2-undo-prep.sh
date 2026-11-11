#!/bin/bash
# Removes what the stopped c2-prep.sh made: the empty ~/evidence-9851c24
# and ~/results-backup-20261110, whose files are checked equal to the
# results files they copy first.
set -eu
rmdir ~/evidence-9851c24
for f in ~/results-backup-20261110/*; do
    n=$(basename "$f")
    if [ -f ~/p8/mutation/$n ]; then cmp "$f" ~/p8/mutation/$n; else cmp "$f" ~/$n; fi
done
rm ~/results-backup-20261110/*.json
rmdir ~/results-backup-20261110
echo "removed; ~/p8 at $(git -C ~/p8 rev-parse --short HEAD)"
