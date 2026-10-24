#!/bin/bash
# cargo deny and cargo audit on the commit that adds the two exceptions of
# 2026-10-19, in a clone of its own, and the check that it changes no
# lockfile, manifest or code against the frozen candidate.
# Usage: deny-again.sh <candidate> <commit>
set -u
candidate=$1
commit=$2
out=~/final7-out/deny-again
rm -rf ~/deny7 "$out"
mkdir -p "$out"
git clone -q <winhome>/VSCodeProjects/Monolith ~/deny7 || exit 1
cd ~/deny7 || exit 1
git checkout -q --detach "$commit" || exit 1
{
    echo "commit $(git rev-parse HEAD), candidate $candidate"
    git diff --stat "$candidate" "$commit"
    echo "lockfile, manifests and code against the candidate:"
    git diff --stat "$candidate" "$commit" -- Cargo.lock '*Cargo.toml' crates fuzz/fuzz_targets
    echo "(nothing above this line means no change)"
} > "$out/diff.txt" 2>&1
cargo deny check > "$out/deny.log" 2>&1
echo "deny exit $?" >> "$out/summary.txt"
cargo audit > "$out/audit.log" 2>&1
echo "audit exit $?" >> "$out/summary.txt"
echo done >> "$out/summary.txt"
