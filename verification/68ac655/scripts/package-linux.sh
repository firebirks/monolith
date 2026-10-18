#!/bin/bash
# Builds the Linux package of the frozen commit from the WSL clone
# ~/final6 (the package script needs a git checkout: it takes the
# repository root and the commit time from git), and copies the archive,
# its checksum and the log to ~/final6-out/package.
set -u
out=~/final6-out/package
rm -rf "$out"
mkdir -p "$out"
cd ~/final6 || exit 1
{
    echo "commit $(git rev-parse HEAD)"
    export CARGO_BUILD_JOBS=2
    bash packaging/linux/build.sh
    echo "exit $?"
} > "$out/package.log" 2>&1
cp target/package/*.tar.gz target/package/*.sha256 "$out/" 2>/dev/null
ls -la "$out" >> "$out/package.log"
