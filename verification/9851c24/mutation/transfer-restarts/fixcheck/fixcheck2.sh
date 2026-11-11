#!/bin/bash
# The check of the corrected test, second part: v3 (a durable chat that
# waits for two more frames while a file comes in), with the corrected
# test (must fail) and with the test before the correction; and what
# clippy checked in base. Scratch copies under ~/p8t, own target dirs.
set -u
here=$(dirname "$0")
NEW=c896861b66b0296db4ab1598589d69a88954d877
TEST=a_sender_that_ignores_the_window_stages_no_more_than_the_file_and_holds_up_no_chat
root=~/p8t
progress="$here/progress2.log"
note() { echo "$(date +%T) $*" | tee -a "$progress"; }
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
for copy in v3 oldv3; do
    [ -e "$root/$copy" ] && { note "STOP: $root/$copy exists"; exit 2; }
    mkdir "$root/$copy"
    git -C ~/p8 archive "$NEW" | tar -x -C "$root/$copy"
done
(cd "$root/v3" && git apply "$here/test-fix.patch") || { note "STOP: patch"; exit 1; }
python3 "$here/violate.py" "$root/v3" v3 >> "$progress" || { note "STOP: v3"; exit 1; }
python3 "$here/violate.py" "$root/oldv3" v3 >> "$progress" || { note "STOP: oldv3"; exit 1; }
declare -A BIN
for copy in v3 oldv3; do
    out=$(cd "$root/$copy" && CARGO_TERM_COLOR=never cargo test --locked -p monolith-core --test transfer_restarts --no-run 2>&1)
    code=$?
    echo "$out" > "$root/$copy-build.log"
    bin=$(echo "$out" | sed -n 's/.*Executable tests\/transfer_restarts.rs (\(.*\))$/\1/p')
    [ "$code" = 0 ] && [ -n "$bin" ] || { note "STOP: build of $copy exit $code"; tail -20 "$root/$copy-build.log" >> "$progress"; exit 1; }
    BIN[$copy]="$root/$copy/$bin"
    note "$copy built: sha256 $(sha256sum "${BIN[$copy]}" | cut -c1-16)"
done
run() {
    local copy=$1 log=$2
    shift 2
    local started=$SECONDS
    (cd "$root/$copy/crates/monolith-core" && CARGO_MANIFEST_DIR=$PWD CARGO_PKG_NAME=monolith-core \
        timeout 2400 "${BIN[$copy]}" "$@" > "$log" 2>&1)
    echo "$? $((SECONDS - started))"
}
for i in 1 2 3; do
    r=$(run v3 "$root/logs/v3-$i.log" --exact "$TEST")
    note "v3 run $i: exit/seconds $r; $(grep -m1 'panicked at' "$root/logs/v3-$i.log" | sed 's/.*panicked at //')"
    grep -m1 -A3 "panicked at" "$root/logs/v3-$i.log" | sed -n '2,4p' | cut -c1-400 >> "$progress"
done
fails=0
for i in $(seq 1 10); do
    r=$(run oldv3 "$root/logs/oldv3-$i.log" --exact "$TEST")
    [ "${r%% *}" = 0 ] || fails=$((fails + 1))
done
note "oldv3: $fails of 10 failed"
note "clippy in base: $(grep -E 'Checking monolith-core|Finished' "$root/logs/clippy.log" | tr '\n' ' ')"
note "v2 run 1 messages order: $(grep -c . "$root/logs/v2-1.log") lines, $(grep -m1 '^test result' "$root/logs/v2-1.log")"
note "fixcheck2 ended"
