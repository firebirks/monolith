#!/bin/sh
# Run as root. While the mutation lists of the Phase 6 final verification
# last (until <home>/final6-mutation.txt says done, at most three days), drops
# the page cache of the WSL VM when it holds more than 3 GB, so that the VM
# gives its memory back to Windows. Only cached file data is dropped; no
# process is touched.
log=<home>/dropcache6.log
echo "start $(date -Is)" > "$log"
end=$(( $(date +%s) + 3 * 24 * 3600 ))
while [ "$(date +%s)" -lt "$end" ]; do
    if grep -q '^done' <home>/final6-mutation.txt 2>/dev/null; then
        break
    fi
    cache=$(free -m | awk '/^Mem:/ {print $6}')
    if [ "$cache" -gt 3000 ]; then
        sync
        echo 3 > /proc/sys/vm/drop_caches
        echo "$(date +%T) dropped ${cache} MB" >> "$log"
    fi
    sleep 120
done
echo "end $(date -Is)" >> "$log"
