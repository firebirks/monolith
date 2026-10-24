#!/bin/bash
# Corrected C3: the candidate store binary (common::run limit 120 s, from
# ~/p7 at the candidate) and the long one (limit 3600 s, from the scratch
# clone ~/repro-long), each built in a target directory of its own, their
# identities recorded, then each pinned to one core with 16 libtest
# threads, twice. Outputs in ~/repro-out/C3b.
set -u
out=~/repro-out/C3b
rm -rf "$out"
mkdir -p "$out"
log() { echo "$(date -Is) $*" >> "$out/summary.txt"; }
secs() { python3 -c "print(round($2-$1,1))"; }
failed() { grep -E '^test .* FAILED$' "$1" | sed -E 's/^test (.*) \.\.\. FAILED$/\1/' | tr '\n' ' '; }
build() {
    # $1 source root, $2 target directory; prints the store executable.
    ( cd "$1" && CARGO_TARGET_DIR="$2" cargo test -p monolith-core --test store --no-run --message-format=json 2>>"$out/build.log" ) | python3 -c '
import json, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m["target"]["name"] == "store":
        print(m["executable"])'
}
commit=$(git -C ~/p7 rev-parse HEAD)
rm -rf ~/c3-target-candidate ~/c3-target-long
C=$(build ~/p7 ~/c3-target-candidate)
L=$(build ~/repro-long ~/c3-target-long)
for pair in "candidate:$C:$HOME/p7:$HOME/c3-target-candidate" "long:$L:$HOME/repro-long:$HOME/c3-target-long"; do
    IFS=: read -r name bin root target <<< "$pair"
    [ -x "$bin" ] || { log "$name: no binary"; exit 1; }
    log "$name: binary $bin"
    log "$name: target $target; source $root at $(git -C "$root" rev-parse HEAD), changed files: $(git -C "$root" status --porcelain | tr '\n' ' ')"
    log "$name: limit line: $(grep -n 'from_secs(' "$root/crates/monolith-core/tests/common/mod.rs" | head -1)"
    log "$name: mtime $(stat -c '%y' "$bin") size $(stat -c '%s' "$bin") sha256 $(sha256sum "$bin" | cut -d' ' -f1)"
done
[ "$(sha256sum "$C" | cut -d' ' -f1)" != "$(sha256sum "$L" | cut -d' ' -f1)" ] || { log "the two binaries are identical"; exit 1; }
[ "$commit" = "$(git -C ~/repro-long rev-parse HEAD)" ] || { log "the two trees are not at the same commit"; exit 1; }
for pair in "candidate:$C:$HOME/p7" "long:$L:$HOME/repro-long"; do
    IFS=: read -r name bin root <<< "$pair"
    cd "$root/crates/monolith-core" || exit 1
    export CARGO_MANIFEST_DIR=$PWD
    for run in 1 2; do
        s=$(date +%s.%N)
        taskset -c 0 "$bin" --test-threads=16 > "$out/$name-$run.log" 2>&1
        rc=$?
        e=$(date +%s.%N)
        timeouts=$(grep -c 'the test did not end in time' "$out/$name-$run.log")
        others=$(grep 'panicked at' "$out/$name-$run.log" | grep -vc 'common/mod.rs:40')
        log "$name run $run rc=$rc $(secs "$s" "$e")s timeouts=$timeouts other-panics=$others failed: $(failed "$out/$name-$run.log")"
    done
done
log "C3b done"
