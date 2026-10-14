#!/bin/bash
# Mirror only, after the official runs: with CS29 applied to the mirror of
# 91bf22c, runs the supervisor tests of monolith-core alone, to see which
# named tests catch the fault now that tests/runtime.rs is not in front of
# them. The fault file is restored after, with a newer time, and compared.
cd ~/mono || exit 1
SP=<work>
W=<repo>
out=~/final-cs29-named.txt
python3 "$SP/sync.py" || exit 99
echo "mirror of $(git -C "$W" rev-parse HEAD 2>/dev/null || echo unknown), $(date -Is)" > "$out"
python3 - <<'PY'
import os, sys
sys.path.insert(0, os.path.expanduser('~/mono/mutation'))
import faults
item = next(f for f in faults.PHASE4 if f['id'] == 'CS29')
p = os.path.expanduser('~/mono/' + item['path'])
t = open(p).read()
for old, new in item['pairs']:
    assert t.count(old) == 1
    t = t.replace(old, new, 1)
open(p, 'w').write(t)
print('applied CS29 to', item['path'])
PY
export CARGO_TERM_COLOR=never
started=$SECONDS
timeout 1800 cargo test --locked -p monolith-core --test supervisor > ~/final-cs29-named.log 2>&1
echo "exit $? ($((SECONDS - started)) s)" >> "$out"
grep -E "^test .* \.\.\. FAILED|^test result" ~/final-cs29-named.log >> "$out"
f=crates/monolith-core/src/supervisor.rs
cp "$W/$f" "$f"
touch "$f"
cmp "$f" "$W/$f" && echo "restored $f" >> "$out"
echo done >> "$out"
