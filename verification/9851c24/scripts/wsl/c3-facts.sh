#!/bin/bash
# Read only: property catches of gate C3, and where FT29 waits.
here=$(dirname "$0")
P=$(cd "$here/../.." && pwd)
echo "== property_catches on gate C3 (merged) and gate B"
python3 -B "$P/property_catches.py" ~/p8 ~/diag-c3/final/results-C3-merged.json "$P/results/results-B-9851c24.json"
echo "== FT29 on c90d5e5: the binaries in order, and the tests of the last one"
log=<repo>/mutation/work/logs/all-c90d5e5-20261111-082454/FT29
grep -E '^\s+(Running|Doc-tests)' "$log.stderr" | sed 's/^ *//' | cut -c1-120
awk '/^running [0-9]+ tests?$/{n++} {last=n} {lines[n]=lines[n] "\n" $0} END{print lines[last]}' "$log.stdout" | grep -E '^(running|test )' | cut -c1-160
