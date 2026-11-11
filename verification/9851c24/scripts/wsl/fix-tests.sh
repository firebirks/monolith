#!/bin/bash
# The seed test of monolith-ui-model on the three Linux toolchains, in the
# clone ~/p8fix at its commit.
set -u
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS=4
cd ~/p8fix || exit 1
git rev-parse --short HEAD
for tc in 1.95.0 1.85.1 1.99.1; do
    echo "== $tc"
    cargo +"$tc" test -p monolith-ui-model --test fuzz_seeds --locked 2>&1 | grep -E "test result|error"
done
