#!/bin/bash
# The fuzz part of the re-verification of a candidate that changed only
# the ui_present target, its seeds and their test, in the clone ~/p8fix:
# every target compiles, ui_present builds, the input that failed and the
# new seed pass, then a smoke run of 25 s and a long run of 600 s, as the
# final stage runs them. Output to ~/p8fix-out.
# Usage: fix-verify.sh <commit>
set -u
commit=$1
out=~/p8fix-out
rm -rf "$out" ~/p8fix-corpus
mkdir -p "$out" ~/p8fix-corpus/ui_present
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
export CARGO_BUILD_JOBS=4
cd ~/p8fix || exit 1
git -c safe.directory='*' fetch -q <repo> phase-8-file-transfer
git checkout -q --detach "$commit" || exit 1
summary=$out/summary.txt
echo "commit $(git rev-parse HEAD) $(date -Is)" > "$summary"
rustc +nightly --version >> "$summary"
step() {
    local name=$1
    shift
    local started=$SECONDS
    "$@" > "$out/$name.log" 2>&1
    echo "$name exit $? ($((SECONDS - started)) s)" >> "$summary"
}
step fuzz-check bash -c 'cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins --locked'
step fuzz-build bash -c 'cd fuzz && cargo +nightly fuzz build ui_present'
for input in ~/p8fix-inputs/*; do
    step "input-$(basename "$input" | cut -c1-14)" bash -c "cd fuzz && cargo +nightly fuzz run ui_present '$input'"
done
run() {
    local seconds=$1 label=$2
    (cd fuzz && cargo +nightly fuzz run ui_present ~/p8fix-corpus/ui_present seeds/ui_present -- \
        -max_total_time="$seconds" -malloc_limit_mb=64 -rss_limit_mb=2048 -timeout=5) \
        > "$out/fuzz-$label-ui_present.log" 2>&1
    local status=$?
    local done crashes
    done=$(grep -E '^Done [0-9]+ runs' "$out/fuzz-$label-ui_present.log" | tail -1)
    crashes=$(grep -cE 'ERROR: libFuzzer|panicked|deadly signal' "$out/fuzz-$label-ui_present.log")
    echo "$label ui_present exit=$status $done crash_lines=$crashes" >> "$summary"
}
run 25 smoke
run 600 long
echo "done $(date -Is)" >> "$summary"
