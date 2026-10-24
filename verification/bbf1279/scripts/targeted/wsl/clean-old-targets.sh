#!/bin/bash
# Removes the old build output in WSL that no run uses: the scratch target
# directories of the reproductions (~/*-target, ~/*-target-*), the scratch clone
# ~/repro-long and a target directory of the clone ~/p7 itself. Never
# mutation/work, which the mutation run uses. The disk of WSL does not
# shrink on Windows when files go; this keeps it from growing.
for dir in ~/*-target ~/*-target-* ~/repro-long ~/p7/target; do
    [ -d "$dir" ] || continue
    rm -rf "$dir" && echo "removed $dir"
done
df -h / | tail -1
