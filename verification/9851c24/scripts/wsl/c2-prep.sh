#!/bin/bash
# Gate C on c896861 (owner, 2026-11-10), before anything runs: brings
# c896861 into ~/p8 from the Windows repository (a local fetch), checks
# that it changes nothing outside mutation/ from 9851c24, keeps a copy of
# every results file and of the transfer_restarts binaries of the gate C
# that failed (the runner removes the copies built from 9851c24 when it
# prepares them for c896861), checks the manifest of the 96 and the disk,
# and checks out c896861. Stops at the first check that does not hold.
set -u
NEW=c896861b66b0296db4ab1598589d69a88954d877
OLD=9851c24b9814092c2a50efdc6da51d99e0633460
ids_file=$1
here=$(dirname "$0")
fail() { echo "STOP: $*"; exit 1; }
cd ~/p8 || fail "no ~/p8"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "~/p8 has changes"
pgrep -af 'mutation/run.py|cargo |rustc' | grep -v pgrep && fail "a runner or a build is running"
git fetch -q origin phase-8-file-transfer || fail "fetch"
[ "$(git rev-parse "$NEW^{commit}")" = "$NEW" ] || fail "no $NEW"
[ "$(git rev-parse "$NEW^")" = "$OLD" ] || fail "the parent of $NEW is not $OLD"
echo "== files changed $OLD..$NEW"
git diff --name-only "$OLD" "$NEW"
outside=$(git diff --name-only "$OLD" "$NEW" -- . ':(exclude)mutation/')
[ -z "$outside" ] || fail "changes outside mutation/: $outside"
for p in Cargo.toml Cargo.lock crates fuzz tests docs rust-toolchain.toml .cargo mutation/faults.py; do
    a=$(git rev-parse "$OLD:$p") b=$(git rev-parse "$NEW:$p")
    [ "$a" = "$b" ] || fail "$p differs"
    echo "same $p ${a:0:12}"
done

echo "== results kept in ~/results-backup-20261110"
[ -e ~/results-backup-20261110 ] && fail "~/results-backup-20261110 exists"
mkdir ~/results-backup-20261110
cp -p ~/p8/mutation/results-*.json ~/results-*.json ~/results-backup-20261110/
(cd ~/results-backup-20261110 && sha256sum * | cut -c1-16,65-)

echo "== binaries of the gate C that failed kept in ~/evidence-9851c24"
[ -e ~/evidence-9851c24 ] && fail "~/evidence-9851c24 exists"
mkdir ~/evidence-9851c24
for t in tree0 tree1; do
    [ "$(cut -c1-40 ~/p8/mutation/work/$t/.mutation-commit)" = "$OLD" ] || fail "$t is not at $OLD"
    # Every build of it: that of the unmodified run, and those of the
    # faults whose tests build another set of crates.
    for bin in $(ls ~/p8/mutation/work/$t/target/debug/deps/transfer_restarts-* | grep -v '\.d$'); do
        cp -p "$bin" ~/evidence-9851c24/$t-$(basename "$bin")
    done
    want=$(grep -o '"transfer_restarts-[^"]*": "[0-9a-f]*' ~/diag-gatec/cycle3/$t-unmodified.binaries.json)
    name=$(echo "$want" | cut -d'"' -f2) sum=$(echo "$want" | cut -d'"' -f4)
    [ "$(sha256sum ~/evidence-9851c24/$t-$name | cut -d' ' -f1)" = "$sum" ] || fail "$t: the copy of $name is not the binary the unmodified run ran"
done
cp -p ~/diag-gatec/cycle3/tree0-unmodified.binaries.json ~/evidence-9851c24/tree0-binaries-9851c24.json
cp -p ~/diag-gatec/cycle3/tree1-unmodified.binaries.json ~/evidence-9851c24/tree1-binaries-9851c24.json
(cd ~/evidence-9851c24 && sha256sum * | cut -c1-16,65-)

git checkout -q --detach "$NEW" || fail "checkout"
[ "$(git rev-parse HEAD)" = "$NEW" ] || fail "not at $NEW"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "~/p8 has changes after the checkout"
echo "== ~/p8 at $(git log --oneline -1)"

echo "== manifest"
python3 - "$ids_file" <<'PY' || fail "manifest"
import json, sys
sys.path.insert(0, "<repo>/mutation")
import faults
ids = open(sys.argv[1]).read().split()
lists = {name: getattr(faults, name) for name in
         ("PHASE1", "PHASE2", "PHASE3", "PHASE4", "PHASE5", "PHASE6", "PHASE7", "PHASE8")}
entry = {item["id"]: (name, item) for name, items in lists.items() for item in items}
problems = []
if len(ids) != 96 or len(set(ids)) != 96:
    problems.append(f"{len(ids)} ids, {len(set(ids))} unique")
for fid in ids:
    if fid not in entry:
        problems.append(f"{fid} is in no list run by `all`")
    elif entry[fid][1]["expect"] != "caught":
        problems.append(f"{fid} is expected to survive")
gate_b = {r["id"] for r in json.load(open("<home>/results-B-9851c24.json"))["results"]}
phase8 = {r["id"] for r in json.load(open("<home>/results-phase8-9851c24-part2.json"))["results"]}
if set(ids) & gate_b:
    problems.append(f"already in gate B: {sorted(set(ids) & gate_b)}")
if set(ids) & phase8:
    problems.append(f"already in the Phase 8 list: {sorted(set(ids) & phase8)}")
for fid in ("MR25", "K4", "CS30", "Q1"):
    if ids.count(fid) != 1:
        problems.append(f"{fid} appears {ids.count(fid)} times")
count = {}
for fid in ids:
    count[entry[fid][0]] = count.get(entry[fid][0], 0) + 1
print(f"96 ids, {len(set(ids))} unique; by list {count}; gate B had {len(gate_b)}, none of them here")
print("MR25, K4, CS30, Q1 once each; every one expected caught")
for problem in problems:
    print("PROBLEM", problem)
sys.exit(1 if problems else 0)
PY

echo "== disk"
df -h / | tail -1
echo "prep done $(date -Is)"
