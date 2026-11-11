#!/bin/bash
# Writes c3r-unmodified.py and checks the syntax of the scripts of the
# resumed gate C3; read only otherwise.
here=$(dirname "$0")
python3 -B "$here/make-c3r.py" || exit 1
python3 -B -m py_compile "$here/c3r-unmodified.py" && echo "c3r-unmodified.py: syntax ok"
rm -rf "$here/__pycache__"
for s in c3r-unmodified.sh targeted-c3r.sh; do bash -n "$here/$s" && echo "$s: syntax ok"; done
diff "$here/c3-unmodified.py" "$here/c3r-unmodified.py"
for t in tree0 tree1; do
    python3 -c "import json,sys; d=json.load(open(sys.argv[1])); print(sys.argv[1], len(d), 'binaries')" ~/diag-c3/$t-binaries.json
done
ls ~/diag-c3r ~/final8-mutation-C3r.log <repo>/mutation/results-all-c90d5e5.json 2>&1 | head -3
