#!/bin/bash
# Read only: the state of ~/p8 and of the runner copies after the stress.
cd ~/p8 || exit 1
echo "~/p8 HEAD $(git rev-parse --short HEAD), tracked files changed: $(git status --porcelain --untracked-files=no | wc -l)"
for t in tree0 tree1; do
    echo "$t marker $(cut -c1-7 ~/p8/mutation/work/$t/.mutation-commit), regression files $(find ~/p8/mutation/work/$t/crates -name '*.proptest-regressions' | wc -l)"
done
echo "binary sha256 $(sha256sum ~/p8/mutation/work/tree1/target/debug/deps/transfer_restarts-9a2146a26bc681d6 | cut -c1-16)"
pgrep -af 'transfer_restarts|iosample|stress-restarts' | grep -v pgrep || echo "nothing left running"
du -sh ~/diag-stress
