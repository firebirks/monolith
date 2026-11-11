#!/bin/bash
# The check of the corrected test (owner, 2026-11-10), in scratch copies
# under ~/p8t, each with its own target directory; nothing of ~/p8 or of
# the repository is changed.
#   base   c896861 with the corrected test
#   v1     base, and a session that sends no chat while a file comes in
#   v2     base, and a durable chat sent two frames late while a file comes in
#   oldv2  c896861 (the test before), and the violation of v2
# Then: the corrected test alone 20 times, the whole binary once, the
# binary as 4 processes at once (2 rounds), the test as 8 processes at
# once (5 rounds), fmt and clippy on base; the test on v1 and v2 (each
# must fail); the earlier test 10 times on oldv2.
set -u
here=$(dirname "$0")
NEW=c896861b66b0296db4ab1598589d69a88954d877
TEST=a_sender_that_ignores_the_window_stages_no_more_than_the_file_and_holds_up_no_chat
root=~/p8t
progress="$here/progress.log"
note() { echo "$(date +%T) $*" | tee -a "$progress"; }
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ -e "$root" ] && { note "STOP: $root exists"; exit 2; }
mkdir -p "$root"
note "fixcheck started"

for copy in base v1 v2 oldv2; do
    mkdir "$root/$copy"
    git -C ~/p8 archive "$NEW" | tar -x -C "$root/$copy"
done
for copy in base v1 v2; do
    (cd "$root/$copy" && git apply "$here/test-fix.patch") || { note "STOP: patch on $copy"; exit 1; }
done
python3 "$here/violate.py" "$root/v1" v1 >> "$progress" || { note "STOP: v1"; exit 1; }
python3 "$here/violate.py" "$root/v2" v2 >> "$progress" || { note "STOP: v2"; exit 1; }
python3 "$here/violate.py" "$root/oldv2" v2 >> "$progress" || { note "STOP: oldv2"; exit 1; }
# What each copy changes from the commit, by file.
fresh=$(mktemp -d)
git -C ~/p8 archive "$NEW" | tar -x -C "$fresh"
for copy in base v1 v2 oldv2; do
    note "$copy changes: $(diff -rq "$fresh" "$root/$copy" | sed "s|$fresh/||;s|$root/$copy/||" | tr '\n' ';')"
done
rm -rf "$fresh"

declare -A BIN
for copy in base v1 v2 oldv2; do
    started=$SECONDS
    out=$(cd "$root/$copy" && CARGO_TERM_COLOR=never cargo test --locked -p monolith-core --test transfer_restarts --no-run 2>&1)
    code=$?
    echo "$out" > "$root/$copy-build.log"
    bin=$(echo "$out" | sed -n 's/.*Executable tests\/transfer_restarts.rs (\(.*\))$/\1/p')
    [ "$code" = 0 ] && [ -n "$bin" ] || { note "STOP: build of $copy exit $code"; tail -20 "$root/$copy-build.log" >> "$progress"; exit 1; }
    BIN[$copy]="$root/$copy/$bin"
    note "$copy built in $((SECONDS - started)) s: $(basename "$bin") sha256 $(sha256sum "${BIN[$copy]}" | cut -c1-16)"
done

# One run of the test binary of `copy`, as cargo runs it, from the crate
# directory; extra arguments go to the test harness.
run() {
    local copy=$1 log=$2
    shift 2
    local started=$SECONDS
    (cd "$root/$copy/crates/monolith-core" && CARGO_MANIFEST_DIR=$PWD CARGO_PKG_NAME=monolith-core \
        timeout 2400 "${BIN[$copy]}" "$@" > "$log" 2>&1)
    local code=$?
    echo "$code $((SECONDS - started))"
}

mkdir -p "$root/logs"
note "== base: the corrected test alone, 20 times"
fails=0
for i in $(seq 1 20); do
    r=$(run base "$root/logs/base-alone-$i.log" --exact "$TEST")
    [ "${r%% *}" = 0 ] || fails=$((fails + 1))
    echo "run $i: exit/seconds $r $(grep -m1 '^test result' "$root/logs/base-alone-$i.log")" >> "$progress"
done
note "base alone: $fails of 20 failed"

note "== base: the whole binary once"
r=$(run base "$root/logs/base-whole.log")
note "base whole: exit/seconds $r; $(grep '^test result' "$root/logs/base-whole.log")"

note "== base: the whole binary as 4 processes at once, 2 rounds"
for round in 1 2; do
    pids=()
    for p in 1 2 3 4; do
        (r=$(run base "$root/logs/base-par4-r$round-p$p.log"); echo "$r" > "$root/logs/base-par4-r$round-p$p.exit") &
        pids+=($!)
    done
    wait "${pids[@]}"
    line="round $round:"
    for p in 1 2 3 4; do
        line="$line p$p $(cat "$root/logs/base-par4-r$round-p$p.exit") $(grep -c "$TEST ... ok" "$root/logs/base-par4-r$round-p$p.log")ok;"
        line="$line $(grep -m1 '^test result' "$root/logs/base-par4-r$round-p$p.log" | cut -c1-40);"
    done
    note "$line"
done

note "== base: the corrected test as 8 processes at once, 5 rounds"
for round in 1 2 3 4 5; do
    pids=()
    for p in 1 2 3 4 5 6 7 8; do
        (r=$(run base "$root/logs/base-par8-r$round-p$p.log" --exact "$TEST"); echo "$r" > "$root/logs/base-par8-r$round-p$p.exit") &
        pids+=($!)
    done
    wait "${pids[@]}"
    note "round $round: $(cat "$root"/logs/base-par8-r$round-p*.exit | tr '\n' ' ')"
done

note "== base: fmt and clippy"
(cd "$root/base" && cargo fmt --all --check > "$root/logs/fmt.log" 2>&1); note "fmt exit $?"
(cd "$root/base" && CARGO_TERM_COLOR=never cargo clippy --locked -p monolith-core --all-targets -- -D warnings > "$root/logs/clippy.log" 2>&1)
note "clippy exit $?; $(grep -c '^warning\|^error' "$root/logs/clippy.log") warning/error lines"

for copy in v1 v2; do
    note "== $copy: the corrected test (must fail)"
    for i in 1 2 3; do
        r=$(run "$copy" "$root/logs/$copy-$i.log" --exact "$TEST")
        note "$copy run $i: exit/seconds $r; $(grep -m1 "panicked at" "$root/logs/$copy-$i.log" | sed 's/.*panicked at //')"
        grep -m1 -A1 "panicked at" "$root/logs/$copy-$i.log" | tail -1 | cut -c1-300 >> "$progress"
    done
done

note "== oldv2: the test before the correction, 10 times"
fails=0
for i in $(seq 1 10); do
    r=$(run oldv2 "$root/logs/oldv2-$i.log" --exact "$TEST")
    [ "${r%% *}" = 0 ] || fails=$((fails + 1))
done
note "oldv2: $fails of 10 failed"
note "fixcheck ended"
