#!/bin/bash
# Runs stress-restarts.py into ~/diag-stress (which must not exist yet),
# after writing the conditions it starts in. Changes no source file.
set -u
here=$(dirname "$0")
out=$HOME/diag-stress
[ -e "$out" ] && { echo "$out exists: not overwritten"; exit 2; }
# Each line also goes to stress-progress.log beside the launcher, to be
# followed from Windows.
python3 -u "$here/stress-restarts.py" "$out" 2>&1 | tee "$HOME/diag-stress.python.log" >> "$here/../stress-progress.log"
code=${PIPESTATUS[0]}
mv "$HOME/diag-stress.python.log" "$out/python.log" 2>/dev/null
{
    echo "== uptime"; uptime
    echo "== kernel"; uname -r
    echo "== meminfo"; head -5 /proc/meminfo
    echo "== processes at the end"; ps -eo pid,stat,etime,pcpu,rss,args --sort=-pcpu | head -15 | cut -c1-150
} > "$out/conditions-end.txt" 2>&1
echo "stress exit $code $(date -Is)"
exit $code
