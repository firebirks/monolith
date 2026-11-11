#!/bin/bash
# Gate C on c90d5e5, first step: c3-prep.sh, then c3-unmodified.py into
# ~/diag-c3. Every line also goes to c3-progress.log beside the launcher.
# Usage: c3-unmodified.sh <ids file>
set -u
here=$(dirname "$0")
progress="$here/../c3-progress.log"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ -e ~/diag-c3 ] && { echo "~/diag-c3 exists" >> "$progress"; exit 2; }
bash "$here/c3-prep.sh" "$1" 2>&1 | tee ~/diag-c3-prep.log >> "$progress"
[ "${PIPESTATUS[0]}" = 0 ] || { echo "prep failed: stop" >> "$progress"; exit 1; }
python3 -u "$here/c3-unmodified.py" "$1" ~/diag-c3 2>&1 | tee ~/diag-c3.python.log >> "$progress"
code=${PIPESTATUS[0]}
mv ~/diag-c3-prep.log ~/diag-c3.python.log ~/diag-c3/ 2>/dev/null
echo "unmodified step exit $code $(date -Is)" >> "$progress"
exit $code
