#!/bin/bash
# Runs, in one runner copy, the transfer_restarts test binary the
# unmodified run of gate C built last, as cargo runs it (from the crate's
# directory), bounded, and prints its failures. Changes no source file.
# Usage: diag-restarts.sh <tree> <label> [bound seconds]
set -u
tree=$HOME/p8/mutation/work/$1
label=$2
bound=${3:-1800}
bin=$(ls -t "$tree"/target/debug/deps/transfer_restarts-* 2>/dev/null | grep -v '\.d$' | head -1)
[ -x "$bin" ] || { echo "no binary in $tree"; exit 1; }
echo "binary $bin $(stat -c %y "$bin" | cut -c1-19) sha256 $(sha256sum "$bin" | cut -c1-16)"
log=$HOME/diag-restarts-$label.log
cd "$tree/crates/monolith-core" || exit 1
started=$SECONDS
CARGO_MANIFEST_DIR=$PWD CARGO_PKG_NAME=monolith-core timeout "$bound" "$bin" > "$log" 2>&1
echo "exit $? in $((SECONDS - started)) s, log $log"
grep -E "^test result|FAILED|panicked|failures:|minimal failing|Test failed" "$log" | head -30
ls "$tree"/crates/monolith-core/tests/*.proptest-regressions 2>/dev/null
