#!/bin/bash
# Gate C on c90d5e5 (owner, 2026-11-10: 98 faults, the 96 and MR26, FT29),
# before anything runs: brings c90d5e5 into ~/p8 from the Windows
# repository (a local fetch), checks that from 9851c24 it changes only the
# runner, the corrected test and DESIGN_QUESTIONS, and nothing of the
# production code or its dependencies; checks the manifest of the 98, that
# no results of this commit exist, and the disk; checks out c90d5e5.
# Stops at the first check that does not hold.
set -u
NEW=c90d5e54735542764fe46855ad68cbfd342273dc
BASE=9851c24b9814092c2a50efdc6da51d99e0633460
RUNNER=c896861b66b0296db4ab1598589d69a88954d877
ids_file=$1
fail() { echo "STOP: $*"; exit 1; }
cd ~/p8 || fail "no ~/p8"
[ -z "$(git status --porcelain --untracked-files=no)" ] || fail "~/p8 has changes"
pgrep -af 'mutation/run.py|cargo |rustc' | grep -v pgrep && fail "a runner or a build is running"
git fetch -q origin phase-8-file-transfer || fail "fetch"
[ "$(git rev-parse "$NEW^{commit}")" = "$NEW" ] || fail "no $NEW"
[ "$(git rev-parse "$NEW^")" = "$RUNNER" ] && [ "$(git rev-parse "$RUNNER^")" = "$BASE" ] || fail "not 9851c24 -> c896861 -> c90d5e5"
echo "== files changed 9851c24..c90d5e5"
changed=$(git diff --name-only "$BASE" "$NEW")
echo "$changed"
expected="crates/monolith-core/tests/transfer_restarts.rs
docs/DESIGN_QUESTIONS.md
mutation/README.md
mutation/affected.py
mutation/run.py
mutation/test_run.py"
[ "$changed" = "$expected" ] || fail "other files changed"
for p in Cargo.toml Cargo.lock fuzz tests rust-toolchain.toml .cargo mutation/faults.py; do
    a=$(git rev-parse "$BASE:$p") b=$(git rev-parse "$NEW:$p")
    [ -n "$a" ] && [ "$a" = "$b" ] || fail "$p differs"
done
n=0
for c in $(git ls-tree --name-only "$NEW" crates/); do
    for p in $(git ls-tree --name-only "$NEW" "$c/"); do
        if [ "$p" = crates/monolith-core/tests ]; then
            for f in $(git ls-tree -r --name-only "$NEW" "$p/"); do
                [ "$f" = crates/monolith-core/tests/transfer_restarts.rs ] && continue
                [ "$(git rev-parse "$BASE:$f")" = "$(git rev-parse "$NEW:$f")" ] || fail "$f differs"
                n=$((n + 1))
            done
            continue
        fi
        a=$(git rev-parse "$BASE:$p") b=$(git rev-parse "$NEW:$p")
        [ -n "$a" ] && [ "$a" = "$b" ] || fail "$p differs"
        n=$((n + 1))
    done
done
[ "$(git ls-tree --name-only "$BASE" crates/)" = "$(git ls-tree --name-only "$NEW" crates/)" ] || fail "the crates differ"
echo "same as 9851c24: Cargo.toml, Cargo.lock, fuzz, tests, toolchain, .cargo, faults.py, and $n paths of the $(git ls-tree --name-only "$NEW" crates/ | wc -l) crates (every one but transfer_restarts.rs)"

[ -e mutation/results-all-c90d5e5.json ] && fail "results of c90d5e5 exist"
[ -e ~/final8-mutation-C3.log ] && fail "~/final8-mutation-C3.log exists"
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
if len(ids) != 98 or len(set(ids)) != 98:
    problems.append(f"{len(ids)} ids, {len(set(ids))} unique")
for fid in ids:
    if fid not in entry:
        problems.append(f"{fid} is in no list run by `all`")
    elif entry[fid][1]["expect"] != "caught":
        problems.append(f"{fid} is expected to survive")
gate_b = {r["id"] for r in json.load(open("<home>/results-B-9851c24.json"))["results"]}
phase8 = {r["id"] for r in json.load(open("<home>/results-phase8-9851c24-part2.json"))["results"]}
again = {"MR26", "FT29"}
if (set(ids) & gate_b) != {"MR26"}:
    problems.append(f"in gate B: {sorted(set(ids) & gate_b)}, expected MR26 only")
if (set(ids) & phase8) != {"FT29"}:
    problems.append(f"in the Phase 8 list: {sorted(set(ids) & phase8)}, expected FT29 only")
for fid in ("MR25", "K4", "CS30", "Q1", "MR26", "FT29"):
    if ids.count(fid) != 1:
        problems.append(f"{fid} appears {ids.count(fid)} times")
run96 = set(open(sys.argv[1].replace("run98", "run96")).read().split())
if set(ids) - run96 != again or len(run96) != 96 or not run96 <= set(ids):
    problems.append("the 98 are not the 96 and MR26, FT29")
count = {}
for fid in ids:
    count[entry[fid][0]] = count.get(entry[fid][0], 0) + 1
print(f"98 ids, {len(set(ids))} unique: the 96 and MR26, FT29; by list {count}")
print("MR26 the only one from gate B, FT29 the only one from the Phase 8 list; every one expected caught")
for problem in problems:
    print("PROBLEM", problem)
sys.exit(1 if problems else 0)
PY

echo "== disk"
df -h / | tail -1
echo "prep done $(date -Is)"
