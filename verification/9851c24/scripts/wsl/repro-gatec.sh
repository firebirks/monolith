#!/bin/bash
# Launches one cycle of repro-gatec.py as targeted-c.sh launches the
# runner: from ~/p8, with the cargo environment. First records how WSL was
# started again, the environment, the cargo configuration and what else
# runs. Usage: repro-gatec.sh <ids file> <cycle> <how WSL was started again>
set -u
out=$HOME/diag-gatec/cycle$2
mkdir -p "$out"
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
{
    echo "== $(date -Is) cycle $2; WSL started again by: $3"
    echo "uptime $(cut -d' ' -f1 /proc/uptime)s; kernel $(uname -r); nproc $(nproc)"
    grep -E '^(MemTotal|MemAvailable|Cached|Dirty|Writeback):' /proc/meminfo
    echo "pressure stall: $(ls /proc/pressure 2>&1 | tr '\n' ' ')"
    echo "== environment (CARGO*, RUST*, PATH)"
    env | grep -E '^(CARGO|RUST|PATH=)' | sort
    grep -Hn "target-dir\|rustc-wrapper\|jobs\|incremental" ~/.cargo/config* ~/p8/.cargo/*.toml ~/p8/mutation/work/tree*/.cargo/*.toml 2>/dev/null \
        || echo "no target-dir, rustc-wrapper, jobs or incremental set"
    for t in tree0 tree1; do
        d=~/p8/mutation/work/$t/target
        echo "$t target: inode $(stat -c %i "$d") symlink $([ -L "$d" ] && echo yes || echo no)"
    done
    echo "== processes"
    ps -eo pid,etime,args --sort=start_time | cut -c1-160
} > "$out/conditions.txt" 2>&1
python3 -u "$(dirname "$0")/repro-gatec.py" "$1" "$2" "$out" > "$out/python.log" 2>&1
code=$?
echo "cycle $2 exit $code $(date -Is)" >> "$HOME/diag-gatec/summary.txt"
exit $code
