#!/bin/bash
# Read-only look at gate C3: the runner, its builds, the machine.
pgrep -af 'mutation/run.py|targeted-c3' | grep -v pgrep | cut -c1-200
echo "cargo/rustc/test processes: $(pgrep -f 'cargo |rustc|/target/' | wc -l)"
uptime
free -g | head -2
df -h / | tail -1
echo "== last lines of the runner log"
tail -3 ~/final8-mutation-C3.log
echo "== newest per-fault logs"
ls -lt --time-style=+%T <repo>/mutation/work/logs/all-c90d5e5-20261110-151711 | head -7
