#!/bin/bash
# Gate C3 after the restart, first step: c3r-unmodified.py into ~/diag-c3r
# (the copies restored by c3r-restore.sh). Lines also go to
# c3r-progress.log beside the launcher.
# Usage: c3r-unmodified.sh <ids file of the 98>
set -u
here=$(dirname "$0")
progress="$here/../c3r-progress.log"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ -e ~/diag-c3r ] && { echo "~/diag-c3r exists" >> "$progress"; exit 2; }
[ -e ~/diag-c3/tree0-binaries.json ] && [ -e ~/diag-c3/tree1-binaries.json ] \
    || { echo "no binaries of ~/diag-c3" >> "$progress"; exit 2; }
[ -e ~/evidence-c3-reboot/tree0-runtime.rs.held ] || { echo "the restore did not run" >> "$progress"; exit 2; }
python3 -u "$here/c3r-unmodified.py" "$1" ~/diag-c3r 2>&1 | tee ~/diag-c3r.python.log >> "$progress"
code=${PIPESTATUS[0]}
mv ~/diag-c3r.python.log ~/diag-c3r/ 2>/dev/null
echo "unmodified step exit $code $(date -Is)" >> "$progress"
exit $code
