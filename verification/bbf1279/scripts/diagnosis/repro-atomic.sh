#!/bin/bash
# Reproduces the intermittent failures of the *_is_atomic tests of
# monolith-core tests/store.rs on the candidate in ~/p7. Nothing in ~/p7
# is changed but mutation/work, which is removed. Outputs in ~/repro-out.
#   A  each test of the binary alone, one thread, timed (atomic ones 3x)
#   B  4 copies of the store binary at once, 3 rounds
#   B2 4 copies of every test binary of monolith-core at once, 1 round
#   C1 the store binary pinned to one core, 2 runs; the invitation test
#      alone pinned to one core, 2 runs
#   C2 as C1, from a scratch clone where the 120 s limit of common::run
#      is 3600 s
set -u
out=~/repro-out
rm -rf "$out"
mkdir -p "$out"
export CARGO_TARGET_DIR=~/repro-target
log() { echo "$(date -Is) $*" >> "$out/summary.txt"; }
secs() { python3 -c "print(round($2-$1,1))"; }
failed() { grep -E '^test .* FAILED$' "$1" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | tr '\n' ' '; }
panics() { echo "== $1" >> "$out/panics.txt"; grep -A4 -E '^---- .* stdout ----$' "$1" >> "$out/panics.txt"; }
executables() {
    # $1: cargo args after "test"; prints the test executables.
    cargo test $1 --no-run --message-format=json 2>>"$out/build.log" | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m.get("profile", {}).get("test"):
        print(m["executable"])'
}

cd ~/p7 || exit 1
log "commit $(git rev-parse HEAD) changed-files=$(git status --porcelain | wc -l) nproc=$(nproc) rustc=$(rustc --version)"
rm -rf mutation/work
B=$(executables "-p monolith-core --test store" | head -1)
log "store binary $B"
[ -x "$B" ] || { log "no store binary"; exit 1; }
command -v taskset > /dev/null || { log "no taskset"; exit 1; }
# Tests run where cargo runs them: in the directory of the crate.
cd crates/monolith-core || exit 1
export CARGO_MANIFEST_DIR=$PWD

# A
"$B" --list 2>/dev/null | sed -n 's/: test$//p' > "$out/tests.txt"
grep -E '_(is|are)_atomic$' "$out/tests.txt" > "$out/atomic.txt"
log "A: $(wc -l < "$out/tests.txt") tests, $(wc -l < "$out/atomic.txt") atomic"
for pass in 1 2 3; do
    list="$out/atomic.txt"
    [ "$pass" = 1 ] && list="$out/tests.txt"
    while read -r t; do
        s=$(date +%s.%N)
        "$B" --exact "$t" --test-threads=1 > "$out/A$pass-$t.log" 2>&1
        rc=$?
        e=$(date +%s.%N)
        log "A$pass $t rc=$rc $(secs "$s" "$e")s"
        [ "$rc" = 0 ] || panics "$out/A$pass-$t.log"
    done < "$list"
done

# B
for round in 1 2 3; do
    for c in 1 2 3 4; do
        ( s=$(date +%s.%N); "$B" > "$out/B$round-$c.log" 2>&1; rc=$?; e=$(date +%s.%N)
          echo "rc=$rc $(secs "$s" "$e")s" > "$out/B$round-$c.rc" ) &
    done
    wait
    for c in 1 2 3 4; do
        log "B$round copy $c $(cat "$out/B$round-$c.rc") failed: $(failed "$out/B$round-$c.log")"
        grep -q ' FAILED$' "$out/B$round-$c.log" && panics "$out/B$round-$c.log"
    done
done

# B2
executables "-p monolith-core" > "$out/core-binaries.txt"
log "B2: $(wc -l < "$out/core-binaries.txt") test binaries"
for c in 1 2 3 4; do
    ( s=$(date +%s.%N)
      while read -r b; do "$b" >> "$out/B2-$c.log" 2>&1; done < "$out/core-binaries.txt"
      e=$(date +%s.%N); echo "$(secs "$s" "$e")s" > "$out/B2-$c.rc" ) &
done
wait
for c in 1 2 3 4; do
    log "B2 copy $c $(cat "$out/B2-$c.rc") failed: $(failed "$out/B2-$c.log")"
    grep -q ' FAILED$' "$out/B2-$c.log" && panics "$out/B2-$c.log"
done

# C1
for run in 1 2; do
    s=$(date +%s.%N)
    taskset -c 0 "$B" > "$out/C1-whole-$run.log" 2>&1
    rc=$?
    e=$(date +%s.%N)
    log "C1 whole run $run rc=$rc $(secs "$s" "$e")s failed: $(failed "$out/C1-whole-$run.log")"
    [ "$rc" = 0 ] || panics "$out/C1-whole-$run.log"
done
for run in 1 2; do
    s=$(date +%s.%N)
    taskset -c 0 "$B" --exact creating_and_revoking_an_invitation_are_atomic > "$out/C1-invitation-$run.log" 2>&1
    rc=$?
    e=$(date +%s.%N)
    log "C1 invitation run $run rc=$rc $(secs "$s" "$e")s"
    [ "$rc" = 0 ] || panics "$out/C1-invitation-$run.log"
done

# C2
rm -rf ~/repro-long
git clone -q --shared ~/p7 ~/repro-long && git -C ~/repro-long checkout -q --detach "$(git rev-parse HEAD)" || { log "clone failed"; exit 1; }
common=~/repro-long/crates/monolith-core/tests/common/mod.rs
before=$(grep -c 'from_secs(120)' "$common")
sed -i '0,/from_secs(120)/s//from_secs(3600)/' "$common"
log "C2: from_secs(120) in common/mod.rs: $before before, $(grep -c 'from_secs(120)' "$common") after; diff: $(git -C ~/repro-long diff --stat | tail -1)"
cd ~/repro-long || exit 1
L=$(executables "-p monolith-core --test store" | head -1)
log "C2 store binary $L"
[ -x "$L" ] || { log "no C2 binary"; exit 1; }
cd crates/monolith-core || exit 1
export CARGO_MANIFEST_DIR=$PWD
for run in 1 2; do
    s=$(date +%s.%N)
    taskset -c 0 "$L" > "$out/C2-whole-$run.log" 2>&1
    rc=$?
    e=$(date +%s.%N)
    log "C2 whole run $run rc=$rc $(secs "$s" "$e")s failed: $(failed "$out/C2-whole-$run.log")"
    [ "$rc" = 0 ] || panics "$out/C2-whole-$run.log"
done
for run in 1 2; do
    s=$(date +%s.%N)
    taskset -c 0 "$L" --exact creating_and_revoking_an_invitation_are_atomic > "$out/C2-invitation-$run.log" 2>&1
    rc=$?
    e=$(date +%s.%N)
    log "C2 invitation run $run rc=$rc $(secs "$s" "$e")s"
    [ "$rc" = 0 ] || panics "$out/C2-invitation-$run.log"
done
log "done"
