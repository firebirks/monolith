#!/bin/bash
# The WSL part of the final verification of Phase 6 on commit $1: puts the
# clones ~/final6 (WSL stage) and ~/mrun (mutation lists) at the commit,
# fetched from the Windows checkout, then runs the WSL stage and the
# mutation lists with two workers. A line per part goes to
# ~/final6-chain.log.
set -u
trap '' HUP
commit=$1
V=<work>
repo=<winhome>/VSCodeProjects/Monolith
log=~/final6-chain.log
echo "chain $(date -Is) for $commit" > "$log"
for clone in ~/final6 ~/mrun; do
    if [ ! -d "$clone/.git" ]; then
        git clone -q "$repo" "$clone" || { echo "clone $clone failed" >> "$log"; exit 1; }
    fi
    git -C "$clone" fetch -q "$repo" phase-6-desktop-client --tags || { echo "fetch $clone failed" >> "$log"; exit 1; }
    git -C "$clone" checkout -q --detach "$commit" || { echo "checkout $clone failed" >> "$log"; exit 1; }
    git -C "$clone" clean -qfdx -e target -e mutation/work
    echo "$clone at $(git -C "$clone" rev-parse HEAD)" >> "$log"
done
rm -rf ~/mrun/mutation/work
echo "wsl stage $(date -Is)" >> "$log"
bash "$V/final-wsl.sh"
echo "wsl stage exit $? $(date -Is)" >> "$log"
echo "mutation $(date -Is)" >> "$log"
bash "$V/final-mutation.sh" 2
echo "mutation exit $? $(date -Is)" >> "$log"
echo done >> "$log"
