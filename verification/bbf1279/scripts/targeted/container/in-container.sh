#!/bin/bash
# Copies the tree mounted at /src (an exported commit) to a private work
# directory and runs the given command there.
set -eu
export CARGO_HOME=/vol/cargo
export CARGO_TARGET_DIR=/vol/target
export PATH=/usr/local/cargo/bin:$PATH
rm -rf "$HOME/work"
mkdir -p "$HOME/work"
tar -C /src --exclude=./target --exclude=./fuzz/target --exclude=./fuzz/corpus \
    --exclude=./fuzz/artifacts -cf - . | tar -C "$HOME/work" -xf -
cd "$HOME/work"
exec "$@"
