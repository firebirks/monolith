#!/bin/bash
# Read only: what takes space in the WSL home, and the target dirs in it.
du -sh ~/* ~/.cargo ~/.rustup 2>/dev/null | sort -rh | head -15
echo "== target directories outside the runner copies"
find ~ -maxdepth 4 -type d -name target -not -path '*/mutation/work/*' 2>/dev/null | while read -r d; do echo "$(du -sh "$d" | cut -f1) $d $(stat -c %y "$d" | cut -c1-16)"; done
