#!/bin/bash
# Gate C on c896861, first step: c2-prep.sh, then c2-unmodified.py into
# ~/diag-c2. Every line also goes to c2-progress.log beside the launcher.
# Usage: c2-unmodified.sh <ids file>
set -u
here=$(dirname "$0")
progress="$here/../c2-progress.log"
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
[ -e ~/diag-c2 ] && { echo "~/diag-c2 exists" >> "$progress"; exit 2; }
bash "$here/c2-prep.sh" "$1" 2>&1 | tee ~/diag-c2-prep.log >> "$progress"
[ "${PIPESTATUS[0]}" = 0 ] || { echo "prep exit ${PIPESTATUS[0]}: stop" >> "$progress"; exit 1; }
python3 -u "$here/c2-unmodified.py" "$1" ~/diag-c2 2>&1 | tee ~/diag-c2.python.log >> "$progress"
code=${PIPESTATUS[0]}
mv ~/diag-c2-prep.log ~/diag-c2.python.log ~/diag-c2/ 2>/dev/null
echo "unmodified step exit $code $(date -Is)" >> "$progress"
exit $code
