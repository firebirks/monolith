#!/bin/bash
# Moves the proptest regression files the runner wrote into its copies
# during gate B to ~/proptest-after-B, keeping them, so that each copy
# holds exactly its commit. Moves only the two files named here.
set -eu
dest=$HOME/proptest-after-B
cd "$HOME/p8/mutation/work"
for tree in tree0 tree1; do
    for rel in crates/monolith-core/tests/transfer_restarts.proptest-regressions \
               crates/monolith-protocol/tests/properties.proptest-regressions; do
        src="$tree/$rel"
        [ -n "$rel" ] && [ -f "$src" ] || { echo "not a file: $src"; continue; }
        mkdir -p "$dest/$tree/${rel%/*}"
        mv -n "$src" "$dest/$tree/$rel"
        echo "moved $src"
    done
done
find "$dest" -type f -exec sha256sum {} +
