#!/bin/bash
# Read only: the FAILED lines and the failures sections of one fault of
# gate C3, and the fault itself.
fid=$1
logs=<repo>/mutation/work/logs/all-c90d5e5-20261110-151711
grep -n 'FAILED\|panicked\|^error' "$logs/$fid.stdout" "$logs/$fid.stderr" | cut -c1-300
echo "== failures sections"
awk '/^failures:$/{on=1} on{print} /^test result:/{on=0}' "$logs/$fid.stdout" | head -60 | cut -c1-300
echo "== the fault"
cd <repo>/mutation && python3 -B -c "
import faults, sys
for name in ('PHASE1','PHASE2','PHASE3','PHASE4','PHASE5','PHASE6','PHASE7','PHASE8'):
    for item in getattr(faults, name):
        if item['id'] == sys.argv[1]:
            print(name, item['path'], item['crates'], item['what'])
            for old, new in item['pairs']:
                print('--- old'); print(old); print('+++ new'); print(new)
" "$fid"
