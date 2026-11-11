#!/bin/bash
# Checks stress-restarts.py with stand-in binaries, in a temporary
# directory: one that passes in a second (2 rounds), one that hangs (a
# bound of 3 s). Runs no test binary of the copies.
set -u
here=$(dirname "$0")
tmp=$(mktemp -d)
printf '#!/bin/sh\necho "running 1 test"\necho "test a ... ok"\nsleep 1\necho "test result: ok. 1 passed; 0 failed"\n' > $tmp/pass
printf '#!/bin/sh\necho "running 1 test"\necho "test a has been running for over 60 seconds"\nsleep 300\n' > $tmp/hang
chmod +x $tmp/pass $tmp/hang
for kind in pass hang; do
    sha=$(sha256sum $tmp/$kind | cut -d' ' -f1)
    sed -e "s|^BINARY = .*|BINARY = \"$tmp/$kind\"|" -e "s|^SHA256 = .*|SHA256 = \"$sha\"|" \
        -e "s|^ROUNDS = .*|ROUNDS = 2|" -e "s|^BOUND = .*|BOUND = 3|" \
        -e "s|^HERE = .*|HERE = \"$here\"|" "$here/stress-restarts.py" > $tmp/stress-$kind.py
    echo "===== $kind"
    python3 -u $tmp/stress-$kind.py $tmp/out-$kind; echo "exit $?"
    ls $tmp/out-$kind $tmp/out-$kind/round01 | tr '\n' ' '; echo
    cat $tmp/out-$kind/round01/proc1.json | tr -d '\n' | cut -c1-600; echo
    head -2 $tmp/out-$kind/io.log | cut -c1-200
done
pgrep -af "$tmp" || echo "no stand-in left running"
rm -rf $tmp
