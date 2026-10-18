#!/bin/bash
# The part of the Phase 6 final verification that runs in the container
# (image monolith-verify:phase3: Rust 1.95.0, 1.85.1 and stable, Tor,
# Chutney, strace): the toolchain matrix with the screens' fixtures, the
# fail-closed test, the private Tor network test, the workflows of the
# desktop client by script, the Linux package, and the frame measurements.
# Run by in-container.sh in a copy of the frozen commit. Each step's output
# goes to /out/<step>.log and its exit status to /out/summary.txt.
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

features="--features monolith-ui/fixtures"
step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
step clippy-fixtures cargo clippy --workspace --all-targets --locked $features -- -D warnings
step test-1.95.0 cargo test --workspace --locked $features
step check-1.85.1 cargo +1.85.1 check --workspace --all-targets --locked $features
step test-1.85.1 cargo +1.85.1 test --workspace --locked $features
step test-stable cargo +stable test --workspace --locked $features
step tree-d cargo tree -d --locked
step fail-closed bash tests/network/fail-closed.sh
CHUTNEY_PATH=/opt/chutney RESTART_CYCLES=${RESTART_CYCLES:-3} step two-node bash tests/tor-network/two-node.sh
step scenarios cargo run --release --locked -p monolith-desktop --example scenarios -- "$out/scenarios"
step measure cargo run --release --locked -p monolith-ui --features fixtures --example measure
step screens cargo run --locked -p monolith-ui --features fixtures --example screens -- "$out/screens"
step package bash -c 'bash packaging/linux/build.sh && cp target/package/*.tar.gz target/package/*.sha256 /out/'
echo "done" >> "$out/summary.txt"
