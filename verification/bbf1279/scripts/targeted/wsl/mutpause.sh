#!/bin/bash
# Stops or continues the mutation run of the re-verification in ~/p7:
# run.py and every process below it. While the run is stopped,
# ~/reverify-bbf.paused exists, so that no phase starts in between.
# Each call is one line of ~/reverify-bbf-pauses.txt with the time in
# seconds, for telling which faults were running while it was stopped.
# Usage: mutpause.sh stop|cont [reason]
set -u
case ${1:-} in
stop) sig=STOP ;;
cont) sig=CONT ;;
*) echo "usage: mutpause.sh stop|cont [reason]" >&2; exit 2 ;;
esac
flag=~/reverify-bbf.paused
[ "$sig" = STOP ] && touch "$flag"
all=
for pass in 1 2 3; do
    all=$(pgrep -f 'python3 -u mutation/run.py')
    frontier=$all
    while [ -n "$frontier" ]; do
        next=$(for p in $frontier; do pgrep -P "$p"; done)
        all="$all $next"
        frontier=$next
    done
    [ -n "${all// /}" ] && kill -"$sig" $all 2>/dev/null
done
[ "$sig" = CONT ] && rm -f "$flag"
count=$(echo $all | wc -w)
echo "$sig $(date +%s) $(date -Is) $count processes ${2:-}" >> ~/reverify-bbf-pauses.txt
echo "$sig $count processes"
