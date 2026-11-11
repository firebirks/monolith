#!/bin/bash
log=<repo>/mutation/work/logs/all-c90d5e5-20261111-082454/FT29
grep -oE 'finished in [0-9.]+s' "$log.stdout" | grep -oE '[0-9.]+' | awk '{s+=$1} END{print "16 binaries before transfer_model: " s " s of tests"}'
grep -E 'Finished `test`|Compiling monolith' "$log.stderr" | tail -3
stat -c '%y stdout' "$log.stdout"; stat -c '%y stderr' "$log.stderr"
