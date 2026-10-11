#!/bin/bash
# Runs mutation phases on the frozen candidate 970316f, in a detached
# worktree of that commit, and keeps the logs and results in this
# directory.
#
# Usage: verification/970316f/run-phases.sh [workers] [phase ...]
# Defaults: 4 workers, phase3 then phase4. Set CPUS (for example 0-10) to
# keep the runs on those CPUs.

set -u
commit=970316fc76d0ea19c1c65239b16556253a2894c8
here=$(cd "$(dirname "$0")" && pwd)
root=$(git -C "$here" rev-parse --show-toplevel) || exit 2
tree=${TREE:-$(dirname "$root")/monolith-970316f}
workers=${1:-4}
shift || true
phases=("$@")
[ ${#phases[@]} -eq 0 ] && phases=(phase3 phase4)
out=$here/stage3

if [ ! -e "$tree" ]; then
    git -C "$root" worktree add --detach "$tree" "$commit" || exit 2
fi
if [ "$(git -C "$tree" rev-parse HEAD)" != "$commit" ] || [ -n "$(git -C "$tree" status --short)" ]; then
    echo "$tree is not a clean checkout of $commit" >&2
    exit 2
fi

limit=()
[ -n "${CPUS:-}" ] && limit=(taskset -c "$CPUS")

echo "start $(date '+%F %T') commit $commit workers $workers phases ${phases[*]}" >>"$out/summary.txt"
for phase in "${phases[@]}"; do
    started=$(date +%s)
    (cd "$tree" && "${limit[@]}" python3 -u mutation/run.py "$phase" "$workers") >"$out/stage3-$phase.log" 2>&1
    code=$?
    cp "$tree/mutation/results-$phase-${commit:0:7}.json" "$out/" 2>/dev/null
    echo "$phase exit=$code $(( ($(date +%s) - started) / 60 ))min $(grep -a '^done:' "$out/stage3-$phase.log" | sed 's/; written to .*//')" >>"$out/summary.txt"
done
echo "end $(date '+%F %T')" >>"$out/summary.txt"
