#!/bin/bash
# D: 4 copies at once, on all 8 cores, of the store binary whose
# common::run limit is 3600 s (scratch clone ~/repro-long). Every 10 s:
# memory and swap of WSL, and for each copy its elapsed time, its CPU
# time, the tests ended so far, and every thread with its state, CPU
# time and wait channel. A copy past 600 s gets two detailed snapshots
# 10 s apart; one past 1500 s gets two more
# 10 s apart and is then killed. Outputs in ~/repro-out/D.
set -u
out=~/repro-out/D
rm -rf "$out"
mkdir -p "$out"
log() { echo "$(date -Is) $*" >> "$out/summary.txt"; }
L=$(sed -n 's/^.* C2 store binary //p' ~/repro-out/summary.txt | sed -n 1p)
[ -x "$L" ] || { log "no binary"; exit 1; }
cd ~/repro-long/crates/monolith-core || exit 1
export CARGO_MANIFEST_DIR=$PWD
log "binary $L nproc=$(nproc) clk_tck=$(getconf CLK_TCK) $(grep -E 'MemTotal|SwapTotal' /proc/meminfo | tr -s ' ' | tr '\n' ' ')"

# Each copy runs under a wrapper that records its pid, then its elapsed
# and CPU time once it ends.
for c in 1 2 3 4; do
    python3 - "$L" "$out/copy$c" <<'EOF' &
import resource, subprocess, sys, time
binary, base = sys.argv[1], sys.argv[2]
start = time.monotonic()
with open(base + ".log", "w") as log:
    child = subprocess.Popen([binary], stdout=log, stderr=subprocess.STDOUT)
    with open(base + ".pid", "w") as f:
        f.write(str(child.pid))
    rc = child.wait()
usage = resource.getrusage(resource.RUSAGE_CHILDREN)
with open(base + ".end", "w") as f:
    f.write(f"rc={rc} elapsed={time.monotonic() - start:.1f}s user={usage.ru_utime:.1f}s sys={usage.ru_stime:.1f}s\n")
EOF
done
sleep 2
start=$(date +%s)

snapshot() {
    # $1 copy, $2 pid, $3 label
    f="$out/copy$1-$3.txt"
    {
        echo "== $(date -Is) copy $1 pid $2 $3"
        cat "/proc/$2/status" 2>/dev/null | grep -E 'State|Threads|VmRSS'
        for t in /proc/"$2"/task/*; do
            tid=${t##*/}
            read -r -a st < "$t/stat" 2>/dev/null || continue
            echo "-- $tid $(cat "$t/comm" 2>/dev/null) state=${st[2]} utime=${st[13]} stime=${st[14]} wchan=$(cat "$t/wchan" 2>/dev/null) syscall=$(cut -d' ' -f1 "$t/syscall" 2>/dev/null)"
            cat "$t/stack" 2>/dev/null
        done
    } >> "$f"
}

while :; do
    now=$(date +%s)
    el=$((now - start))
    mem=$(awk '/MemAvailable|SwapFree/ {printf "%s=%dM ", $1, $2/1024}' /proc/meminfo)
    swp=$(awk '/pswpin|pswpout/ {printf "%s=%s ", $1, $2}' /proc/vmstat)
    echo "$(date -Is) t=${el}s $mem$swp" >> "$out/memory.txt"
    alive=0
    for c in 1 2 3 4; do
        [ -e "$out/copy$c.end" ] && continue
        pid=$(cat "$out/copy$c.pid" 2>/dev/null) || continue
        [ -d "/proc/$pid" ] || continue
        alive=$((alive + 1))
        read -r -a st < "/proc/$pid/stat"
        ok=$(grep -c ' \.\.\. ok$' "$out/copy$c.log")
        echo "$(date -Is) t=${el}s copy $c cpu_ticks=$((st[13] + st[14])) tests_ok=$ok" >> "$out/progress.txt"
        {
            echo "== t=${el}s"
            for t in /proc/"$pid"/task/*; do
                read -r -a ts < "$t/stat" 2>/dev/null || continue
                echo "$(cat "$t/comm" 2>/dev/null) ${ts[2]} $((ts[13] + ts[14])) $(cat "$t/wchan" 2>/dev/null)"
            done
        } >> "$out/copy$c-threads.txt"
        if [ "$el" -ge 600 ] && [ ! -e "$out/copy$c.late" ]; then
            snapshot "$c" "$pid" late-1
            sleep 10
            snapshot "$c" "$pid" late-2
            touch "$out/copy$c.late"
            log "copy $c still running at ${el}s: two snapshots taken"
        fi
        if [ "$el" -ge 1500 ] && [ ! -e "$out/copy$c.killed" ]; then
            snapshot "$c" "$pid" final-1
            sleep 10
            snapshot "$c" "$pid" final-2
            kill -9 "$pid"
            touch "$out/copy$c.killed"
            log "copy $c killed at ${el}s after two snapshots"
        fi
    done
    [ "$alive" = 0 ] && break
    sleep 10
done
wait
for c in 1 2 3 4; do
    log "copy $c $(cat "$out/copy$c.end" 2>/dev/null) failed: $(grep -E '^test .* FAILED$' "$out/copy$c.log" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | tr '\n' ' ') panics: $(grep -c 'panicked at' "$out/copy$c.log")"
done
log "D done"
