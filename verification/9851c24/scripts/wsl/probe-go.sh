#!/bin/bash
# The state of the mutation baseline, and the second ui_present input of
# the frozen commit run on the new candidate's build in ~/p8fix.
cat ~/final8-mutation.txt
tail -3 ~/final8-mutation-phase8.log 2>/dev/null
cd ~/p8 && git rev-parse --short HEAD
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"
cp ~/p8/fuzz/artifacts/ui_present/crash-5e7ca67dd2c284c03cd90807717b7737062d6da6 ~/p8fix-inputs/
cd ~/p8fix/fuzz && git rev-parse --short HEAD
cargo +nightly fuzz run ui_present ~/p8fix-inputs/crash-5e7ca67dd2c284c03cd90807717b7737062d6da6 2>&1 | grep -E "panicked|Executed|exit status"
echo "run exit ${PIPESTATUS[0]}"
