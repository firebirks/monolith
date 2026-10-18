#!/bin/sh
# Run as root. While the faults run alone after the Phase 6 final mutation
# lists (until <home>/final6-alone.txt says done, at most one day), drops the
# page cache of the WSL VM when it holds more than 3 GB. Only cached file
# data is dropped; no process is touched.
log=<home>/dropcache-alone6.log
echo "start $(date -Is)" > "$log"
end=$(( $(date +%s) + 24 * 3600 ))
while [ "$(date +%s)" -lt "$end" ]; do
    if grep -q '^done' <home>/final6-alone.txt 2>/dev/null; then
        break
    fi
    cache=$(free -m | awk '/^Mem:/ {print $6}')
    if [ "$cache" -gt 3000 ]; then
        sync
        echo 3 > /proc/sys/vm/drop_caches
        echo "$(date +%T) dropped ${cache} MB" >> "$log"
    fi
    sleep 60
done
echo "end $(date -Is)" >> "$log"
