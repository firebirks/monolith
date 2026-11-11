#!/bin/bash
# Gate C3 after Windows restarted (2026-11-11 00:44, an update): 96 of the
# 98 ended, FT29 and HR77 were running and each copy still holds the fault
# it ran, in crates/monolith-core/src/runtime.rs. This keeps the evidence
# of the 96, saves what each copy holds, puts back the file of c90d5e5 the
# way the runner's restore does (a newer time, so cargo builds it again),
# and checks that every tracked file of both copies is that of c90d5e5.
# Stops at the first check that does not hold.
set -u
NEW=c90d5e54735542764fe46855ad68cbfd342273dc
FILE=crates/monolith-core/src/runtime.rs
ev=~/evidence-c3-reboot
fail() { echo "STOP: $*"; exit 1; }
cd ~/p8 || fail "no ~/p8"
pgrep -af 'mutation/run.py|cargo |rustc' | grep -v pgrep && fail "a runner or a build is running"
[ "$(git rev-parse HEAD)" = "$NEW" ] || fail "~/p8 not at $NEW"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "~/p8 has changes"
[ -e "$ev" ] && fail "$ev exists"
mkdir -p "$ev"

echo "== the lock of the runner"
if [ -e mutation/work/lock ]; then
    pid=$(cat mutation/work/lock)
    echo "mutation/work/lock holds $pid; that process: $(ps -o pid=,comm= -p "$pid" 2>/dev/null || echo 'none')"
else
    echo "no lock"
fi

echo "== the evidence of the 96"
logs=mutation/work/logs/all-c90d5e5-20261110-151711
cp -p ~/final8-mutation-C3.log "$ev/" || fail "copy of the runner log"
(cd "$logs" && sha256sum ./* > "$ev/logs-all-c90d5e5-20261110-151711.sha256") || fail "hashes of the logs"
echo "runner log kept; $(wc -l < "$ev/logs-all-c90d5e5-20261110-151711.sha256") log files hashed in $ev"
[ -e mutation/results-all-c90d5e5.json ] && fail "results-all-c90d5e5.json exists: not expected"

echo "== what each copy holds"
git show "$NEW:$FILE" > "$ev/runtime.rs.c90d5e5"
python3 -B - "$ev" <<'PY' || fail "faults left in the copies"
import sys
sys.path.insert(0, "<repo>/mutation")
import faults
ev = sys.argv[1]
original = open(f"{ev}/runtime.rs.c90d5e5", encoding="utf-8", newline="").read()
items = {i["id"]: i for i in faults.PHASE7 + faults.PHASE8 if i["id"] in ("FT29", "HR77")}
held = {}
for tree in ("tree0", "tree1"):
    text = open(f"<repo>/mutation/work/{tree}/crates/monolith-core/src/runtime.rs",
                encoding="utf-8", newline="").read()
    for fid, item in items.items():
        mutated = original
        for old, new in item["pairs"]:
            mutated = mutated.replace(old, new, 1)
        if text == mutated:
            held[tree] = fid
    print(f"{tree}: holds exactly {held.get(tree, 'NEITHER FT29 NOR HR77')}")
sys.exit(0 if sorted(held.values()) == ["FT29", "HR77"] else 1)
PY
for tree in tree0 tree1; do
    t=mutation/work/$tree
    [ "$(cat "$t/.mutation-commit")" = "$NEW" ] || fail "$tree: marker"
    cp -p "$t/$FILE" "$ev/$tree-runtime.rs.held"
    diff -u "$ev/runtime.rs.c90d5e5" "$t/$FILE" > "$ev/$tree-runtime.rs.diff"
done

echo "== every other tracked file of the copies"
fresh=$(mktemp -d)
git archive "$NEW" | tar -x -C "$fresh"
for tree in tree0 tree1; do
    differs=$(git ls-tree -r --name-only "$NEW" | while read -r f; do
        [ "$f" = "$FILE" ] && continue
        cmp -s "$fresh/$f" "mutation/work/$tree/$f" || echo "$f"
    done)
    [ -z "$differs" ] || { rm -rf "$fresh"; fail "$tree: other files differ: $differs"; }
    echo "$tree: every tracked file but $FILE same as c90d5e5"
done

echo "== restore"
for tree in tree0 tree1; do
    t=mutation/work/$tree
    cp "$ev/runtime.rs.c90d5e5" "$t/$FILE" && touch "$t/$FILE" || fail "$tree: restore"
    cmp -s "$fresh/$FILE" "$t/$FILE" || fail "$tree: not restored"
done
for tree in tree0 tree1; do
    differs=$(git ls-tree -r --name-only "$NEW" | while read -r f; do
        cmp -s "$fresh/$f" "mutation/work/$tree/$f" || echo "$f"
    done)
    [ -z "$differs" ] || { rm -rf "$fresh"; fail "$tree: differs after the restore: $differs"; }
    echo "$tree: every one of $(git ls-tree -r --name-only "$NEW" | wc -l) tracked files same as c90d5e5"
done
rm -rf "$fresh"
df -h / | tail -1
echo "restore done $(date -Is)"
