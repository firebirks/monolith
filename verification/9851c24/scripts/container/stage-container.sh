#!/bin/bash
# The part of the Phase 8 final verification that runs in the container
# (image monolith-verify:phase7: Rust 1.95.0, 1.85.1, the stable of its
# build 1.99.0 and the current stable 1.99.1; Tor, Chutney, strace): the
# toolchain matrix with the screens' fixtures, the documentation, the
# duplicate dependencies, the fail-closed test, the private Tor network
# test with every step (the file transfers among them, and a peer built
# from the tag phase-7-complete for step 31), the workflows by script, the
# file transfers by script, the frame measurements and the pictures of
# every fixture. Run by in-container.sh in a copy of the frozen commit,
# with the tag exported at /src7. Each step's output goes to
# /out/<step>.log and its exit status to /out/summary.txt.
set -u
out=/out
: > "$out/summary.txt"

step() {
    local name=$1
    shift
    local started=$SECONDS
    "$@" > "$out/$name.log" 2>&1
    local status=$?
    echo "$name exit $status ($((SECONDS - started)) s)" >> "$out/summary.txt"
}

# The peer of Phase 7, built from the tag in a target directory of its own.
p7="$HOME/phase7"
rm -rf "$p7"
mkdir -p "$p7"
tar -C /src7 --exclude=./target -cf - . | tar -C "$p7" -xf -
step phase7-build bash -c "cd '$p7' && CARGO_TARGET_DIR=/vol/target7 cargo build --locked -p monolith-cli"
export MONOLITH_PHASE7_BIN=/vol/target7/debug/monolith

{
    echo "commit $(cat COMMIT)"
    echo "phase 7 peer $(cat "$p7/COMMIT") (phase-7-complete), binary sha256 $(sha256sum "$MONOLITH_PHASE7_BIN" | cut -d' ' -f1)"
    rustc --version
    rustc +1.85.1 --version
    rustc +stable --version
    rustc +1.99.1 --version
    tor --version | head -1
    echo "chutney $(cat /opt/chutney.commit)"
    strace -V | head -1
    python3 --version
    uname -sr
    echo "restart cycles ${RESTART_CYCLES:-3}"
} > "$out/environment.txt" 2>&1

features="--features monolith-ui/fixtures"
step fmt cargo fmt --all --check
step clippy cargo clippy --workspace --all-targets --locked -- -D warnings
step clippy-fixtures cargo clippy --workspace --all-targets --locked $features -- -D warnings
step doc env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked $features
step test-1.95.0 cargo test --workspace --locked $features
step check-1.85.1 cargo +1.85.1 check --workspace --all-targets --locked $features
step test-1.85.1 cargo +1.85.1 test --workspace --locked $features
step test-1.99.1 cargo +1.99.1 test --workspace --locked $features
step tree-d cargo tree -d --locked
step fail-closed bash tests/network/fail-closed.sh
CHUTNEY_PATH=/opt/chutney RESTART_CYCLES=${RESTART_CYCLES:-3} step two-node bash tests/tor-network/two-node.sh
step scenarios cargo run --release --locked -p monolith-desktop --example scenarios -- "$out/scenarios"
step files cargo run --release --locked -p monolith-desktop --example files -- "$out/files"
step measure cargo run --release --locked -p monolith-ui --features fixtures --example measure
step screens cargo run --locked -p monolith-ui --features fixtures --example screens -- "$out/screens"
echo "done" >> "$out/summary.txt"
