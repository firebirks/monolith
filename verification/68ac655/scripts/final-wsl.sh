#!/bin/bash
# The part of the Phase 6 final verification that runs in the WSL clone
# ~/final6 at the frozen commit: cargo deny, cargo audit, the identity of
# the commits of the phase, the fuzz targets (build, a smoke run of each,
# long runs of the two Phase 6 added), and the desktop client opened on the
# WSLg display, photographing its own window. Output goes to
# ~/final6-out.
set -u
out=~/final6-out
rm -rf "$out"
mkdir -p "$out"
cd ~/final6 || exit 1
export CARGO_BUILD_JOBS=4
{
    echo "commit $(git rev-parse HEAD)"
    rustc --version
    rustc +nightly --version
    cargo deny --version
    cargo audit --version
    cargo +nightly fuzz --version
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

step deny cargo deny check
step audit cargo audit
step identity bash -c 'git log phase-5-complete..HEAD --format="%an <%ae> | %cn <%ce>" | sort | uniq -c'
step trailers bash -c 'git log phase-5-complete..HEAD --format=%B | grep -ciE "co-authored-by|signed-off-by" || true'
step fuzz-check bash -c 'cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins --locked'
step fuzz-build bash -c 'cd fuzz && cargo +nightly fuzz build'

# The client on the WSLg display: it writes a picture of its window two
# seconds after it opens, and closes.
step gui-build cargo build --release --locked -p monolith-desktop --features screenshot
gui() {
    local name=$1 theme=$2
    rm -rf "$out/gui-$name"
    mkdir -p "$out/gui-$name"
    printf 'theme = %s\n' "$theme" > "$out/gui-$name/desktop.conf"
    MONOLITH_SCREENSHOT="$out/gui-$name.png" timeout 60 target/release/monolith-desktop \
        --data-dir "$out/gui-$name/data" --config "$out/gui-$name/desktop.conf"
}
step gui-light gui light light
step gui-dark gui dark dark

targets=$(sed -n 's/^name = "\(.*\)"$/\1/p' fuzz/Cargo.toml | grep -v '^monolith-fuzz$')
corpus=~/final6-corpus
rm -rf "$corpus"
run() {
    local target=$1 seconds=$2 label=$3 status done crashes
    mkdir -p "$corpus/$target"
    (cd fuzz && cargo +nightly fuzz run "$target" "$corpus/$target" "seeds/$target" -- \
        -max_total_time="$seconds" -malloc_limit_mb=64 -rss_limit_mb=2048 -timeout=5) \
        > "$out/fuzz-$label-$target.log" 2>&1
    status=$?
    done=$(grep -E '^Done [0-9]+ runs' "$out/fuzz-$label-$target.log" | tail -1)
    crashes=$(grep -cE 'ERROR: libFuzzer|panicked|deadly signal' "$out/fuzz-$label-$target.log")
    echo "$label $target exit=$status $done crash_lines=$crashes" >> "$out/fuzz-summary.txt"
}
for target in $targets; do
    run "$target" 25 smoke
done
for target in ui_present ui_model_sequence; do
    run "$target" 600 long
done
echo "done" >> "$out/fuzz-summary.txt"
echo "done" >> "$out/summary.txt"
