#!/bin/bash
# Frees space in the WSL disk that the baseline's copies can take instead
# of growing it: the builds of the stage clone and of the fix clone, which
# the baseline does not use (it builds in ~/p8/mutation/work).
du -sh ~/p8/target ~/p8/fuzz/target ~/p8fix/target ~/p8fix/fuzz/target ~/final8-corpus ~/p8fix-corpus 2>/dev/null
pgrep -af 'p8fix' && { echo "something uses p8fix"; exit 1; }
rm -rf ~/p8/target ~/p8/fuzz/target ~/p8fix/target ~/p8fix/fuzz/target
df -h / | tail -1
