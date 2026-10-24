#!/bin/bash
# The Linux part of the targeted re-verification, in the image
# monolith-verify:phase7: formatting, clippy (with and without the
# screens' fixtures) and the tests of monolith-core on 1.95.0, 1.85.1 and
# 1.99.1. Run by in-container.sh in a copy of the candidate. Each step's
# output goes to /out/<step>.log and its exit status to /out/summary.txt.
set -u
out=/out
: > "$out/summary.txt"
{
    echo "commit $(cat COMMIT)"
    rustc --version
    rustc +1.85.1 --version
    rustc +1.99.1 --version
    uname -sr
} > "$out/environment.txt" 2>&1

step() {
    local name=$1
    shift
    local started=$SECONDS
    "$@" > "$out/$name.log" 2>&1
    local status=$?
    echo "$name exit $status ($((SECONDS - started)) s)" >> "$out/summary.txt"
}

features="--features monolith-ui/fixtures"
step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
step clippy-fixtures cargo clippy --workspace --all-targets --locked $features -- -D warnings
step test-core-1.95.0 cargo test -p monolith-core --locked
step test-core-1.85.1 cargo +1.85.1 test -p monolith-core --locked
step test-core-1.99.1 cargo +1.99.1 test -p monolith-core --locked
echo "done" >> "$out/summary.txt"
