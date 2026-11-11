#!/bin/bash
# A trial of the given faults of one list with the official runner, in the
# clone ~/p8fix at the given commit (its own copies under mutation/work).
# Output to ~/p8fix-fault.log.
# Usage: fix-fault.sh <commit> <list> <id>...
set -u
commit=$1
list=$2
shift 2
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
cd ~/p8fix || exit 1
git -c safe.directory='*' fetch -q <repo> phase-8-file-transfer
git checkout -q --detach "$commit" || exit 1
echo "at $(git rev-parse HEAD) $(date -Is)" > ~/p8fix-fault.log
python3 -u mutation/run.py "$list" 1 "$@" >> ~/p8fix-fault.log 2>&1
echo "exit $? $(date -Is)" >> ~/p8fix-fault.log
