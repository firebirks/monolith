#!/bin/bash
# After repro-atomic.sh: the store binary pinned to one core with the
# threads of libtest forced to 16, so that the tests share that core as
# they share the machine under load (taskset alone makes libtest run one
# test at a time). The candidate binary, then the scratch one where the
# limit of common::run is 3600 s.
set -u
out=~/repro-out
log() { echo "$(date -Is) $*" >> "$out/summary.txt"; }
secs() { python3 -c "print(round($2-$1,1))"; }
failed() { grep -E '^test .* FAILED$' "$1" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | tr '\n' ' '; }
until grep -qE " done$| no | clone failed" "$out/summary.txt"; do sleep 20; done
B=$(sed -n 's/^.* store binary //p' "$out/summary.txt" | sed -n 1p)
L=$(sed -n 's/^.* C2 store binary //p' "$out/summary.txt" | sed -n 1p)
for pair in "C3-candidate:$B:$HOME/p7" "C3-limit3600:$L:$HOME/repro-long"; do
    name=${pair%%:*}
    rest=${pair#*:}
    bin=${rest%%:*}
    root=${rest#*:}
    [ -x "$bin" ] || { log "$name no binary"; continue; }
    cd "$root/crates/monolith-core" || exit 1
    export CARGO_MANIFEST_DIR=$PWD
    for run in 1 2; do
        s=$(date +%s.%N)
        taskset -c 0 "$bin" --test-threads=16 > "$out/$name-$run.log" 2>&1
        rc=$?
        e=$(date +%s.%N)
        timeouts=$(grep -c 'the test did not end in time' "$out/$name-$run.log")
        others=$(grep -E "panicked at" "$out/$name-$run.log" | grep -vc 'common/mod.rs:40')
        log "$name run $run rc=$rc $(secs "$s" "$e")s timeouts=$timeouts other-panics=$others failed: $(failed "$out/$name-$run.log")"
    done
done
log "C3 done"
