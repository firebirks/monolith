#!/bin/bash
# The part of the final verification that runs in the container: the
# toolchain matrix, the fail-closed test and the private Tor network test.
# Run by in-container.sh in a copy of the frozen commit. Each step's
# output goes to /out/<step>.log and its exit status to /out/summary.txt.
set -u
out=/out
: > "$out/summary.txt"
{
    echo "commit $(cat COMMIT)"
    rustc --version
    rustc +1.85.1 --version
    rustc +stable --version
    tor --version | head -1
    echo "chutney $(cat /opt/chutney.commit)"
    strace -V | head -1
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

step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
step test-1.95.0 cargo test --workspace --locked
step check-1.85.1 cargo +1.85.1 check --workspace --all-targets --locked
step test-1.85.1 cargo +1.85.1 test --workspace --locked
step test-stable cargo +stable test --workspace --locked
step tree-d cargo tree -d --locked
step fail-closed bash tests/network/fail-closed.sh
CHUTNEY_PATH=/opt/chutney RESTART_CYCLES=${RESTART_CYCLES:-3} step two-node bash tests/tor-network/two-node.sh
echo "done" >> "$out/summary.txt"
