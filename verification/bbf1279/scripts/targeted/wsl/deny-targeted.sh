#!/bin/bash
# cargo deny and cargo audit on the candidate of the targeted
# re-verification, in a clone of its own, and the proof that its lockfile,
# manifests and resolved dependency graph are those of the full candidate.
# Usage: deny-targeted.sh <full candidate> <candidate>
set -u
base=$1
commit=$2
out=~/targeted-out/deny
rm -rf ~/deny-t "$out"
mkdir -p "$out"
git clone -q <winhome>/VSCodeProjects/Monolith ~/deny-t || exit 1
cd ~/deny-t || exit 1
{
    echo "base $(git rev-parse "$base"), candidate $(git rev-parse "$commit")"
    echo "changed files:"
    git diff --name-status "$base" "$commit"
    echo "lockfile and manifests against the base (nothing below means no change):"
    git diff --stat "$base" "$commit" -- Cargo.lock Cargo.toml '*/Cargo.toml' rust-toolchain.toml .cargo/config.toml
    echo "(end)"
    for c in "$base" "$commit"; do
        echo "Cargo.lock at $c: $(git show "$c:Cargo.lock" | sha256sum | cut -d' ' -f1)"
    done
} > "$out/diff.txt" 2>&1
for c in "$base" "$commit"; do
    git checkout -q --detach "$c" || exit 1
    cargo tree --locked --offline --workspace -e all --target all --prefix depth > "$out/tree-${c:0:7}.txt" 2>&1
    echo "tree $c exit $? $(sha256sum < "$out/tree-${c:0:7}.txt" | cut -d' ' -f1)" >> "$out/summary.txt"
    cargo metadata --locked --offline --format-version 1 | python3 -c 'import json,sys; d=json.load(sys.stdin); print(json.dumps(d["resolve"], sort_keys=True))' | sha256sum | sed "s/-$/resolve $c/" >> "$out/summary.txt"
done
if cmp -s "$out/tree-${base:0:7}.txt" "$out/tree-${commit:0:7}.txt"; then echo "trees identical" >> "$out/summary.txt"; else echo "trees DIFFER" >> "$out/summary.txt"; fi
git checkout -q --detach "$commit"
cargo deny --version >> "$out/summary.txt" 2>&1
cargo deny check > "$out/deny.log" 2>&1
echo "deny exit $?" >> "$out/summary.txt"
cargo audit --version >> "$out/summary.txt" 2>&1
cargo audit > "$out/audit.log" 2>&1
echo "audit exit $?" >> "$out/summary.txt"
echo done >> "$out/summary.txt"
