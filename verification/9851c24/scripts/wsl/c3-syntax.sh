#!/bin/bash
here=$(dirname "$0")
for f in c3-prep.sh c3-unmodified.sh targeted-c3.sh pause-baseline.sh; do bash -n "$here/$f" && echo "ok $f" || echo "BAD $f"; done
python3 -m py_compile "$here/c3-unmodified.py" && echo "ok c3-unmodified.py"
