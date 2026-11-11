#!/bin/bash
# Launches repro-unmodified.py as targeted-c.sh launches the runner: from
# ~/p8, with the cargo environment; first records the shell environment,
# the cargo configuration and where each copy builds.
# Usage: repro-unmodified.sh <ids file> <count>
set -u
out=$HOME/diag-unmod
mkdir -p "$out"
cd ~/p8 || exit 1
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
{
    echo "== $(date -Is) environment (CARGO*, RUST*, PATH)"
    env | grep -E '^(CARGO|RUST|PATH=)' | sort
    echo "== cargo configuration files"
    ls -la ~/.cargo/ ~/p8/.cargo/ ~/p8/mutation/work/tree0/.cargo/ ~/p8/mutation/work/tree1/.cargo/ 2>&1
    grep -Hn "target-dir\|rustc-wrapper\|jobs\|incremental" ~/.cargo/config* ~/p8/.cargo/*.toml ~/p8/mutation/work/tree*/.cargo/*.toml 2>/dev/null \
        || echo "no target-dir, rustc-wrapper, jobs or incremental set"
    echo "== target directories"
    for t in tree0 tree1; do
        d=~/p8/mutation/work/$t/target
        echo "$t: $(readlink -f "$d") inode $(stat -c %i "$d") symlink $([ -L "$d" ] && echo yes || echo no)"
    done
    echo "== transfer_restarts binaries"
    for t in tree0 tree1; do
        for b in ~/p8/mutation/work/$t/target/debug/deps/transfer_restarts-*; do
            case $b in *.d) continue ;; esac
            echo "$b inode $(stat -c %i "$b") links $(stat -c %h "$b") $(stat -c %y "$b" | cut -c1-19) $(sha256sum "$b" | cut -c1-64)"
        done
    done
    echo "== nproc $(nproc); other cargo, runner or test processes:"
    pgrep -af "cargo|run.py|/deps/" | grep -v pgrep || echo none
} > "$out/conditions.txt" 2>&1
python3 -u "$(dirname "$0")/repro-unmodified.py" "$1" "$2" "$out" > "$out/python.log" 2>&1
echo "exit $? $(date -Is)" >> "$out/summary.txt"
echo "done repro" >> "$out/summary.txt"
