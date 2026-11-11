#!/bin/bash
# A clone of its own (~/p8fix, with its own fuzz target directory) at the
# given commit, and the ui_present fuzz target built in it; then the given
# inputs run once each. Output to ~/p8fix-<label>.log.
# Usage: fix-build.sh <commit> <label> [input...]
set -u
commit=$1
label=$2
shift 2
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS=4
log=~/p8fix-$label.log
{
    if [ ! -d ~/p8fix/.git ]; then
        git -c safe.directory='*' clone -q <repo> ~/p8fix || exit 1
    fi
    cd ~/p8fix || exit 1
    git -c safe.directory='*' fetch -q <repo> phase-8-file-transfer
    git checkout -q --detach "$commit" || exit 1
    echo "at $(git rev-parse HEAD)"
    cd fuzz || exit 1
    cargo +nightly fuzz build ui_present 2>&1 | tail -3
    echo "build exit ${PIPESTATUS[0]}"
    for input in "$@"; do
        echo "--- $input"
        cargo +nightly fuzz run ui_present "$input" 2>&1 | grep -E "panicked|left:|right:|Executed|exit status|Done" | head -8
        echo "run exit ${PIPESTATUS[0]}"
    done
} > "$log" 2>&1
echo "finished" >> "$log"
