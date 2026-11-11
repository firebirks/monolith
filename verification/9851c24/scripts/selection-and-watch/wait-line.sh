#!/bin/bash
# Waits until `file` has a line matching `pattern`, keeping the RAM
# watchdog meanwhile (the WSL page cache goes back to Windows below 4 GB
# free; below 2 GB after that the baseline is paused, exit 3), then
# prints the last lines of `file`.
# Usage: wait-line.sh <file> <pattern> [disk alert GB, default 50]
file=$1 pattern=$2 disk_min=${3:-50}
S="<work>"
until grep -qE "$pattern" "$file" 2>/dev/null; do
    free=$(powershell.exe -NoProfile -Command "[math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory/1MB,1)" 2>/dev/null | tr -d '\r' | tr ',' '.')
    if [ -n "$free" ] && awk "BEGIN{exit !($free < 4)}"; then
        MSYS_NO_PATHCONV=1 wsl.exe -d Debian -u root -- sh "/mnt/c${S#/c}/dropcache.sh" >/dev/null 2>&1
        echo "$(date +%T) dropped the WSL cache at ${free} GB free" >> "$S/watchdog.log"
        sleep 5
        free=$(powershell.exe -NoProfile -Command "[math]::Round((Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory/1MB,1)" 2>/dev/null | tr -d '\r' | tr ',' '.')
        # The owner's rule: below 2 GB free the baseline is paused.
        if [ -n "$free" ] && awk "BEGIN{exit !($free < 2)}"; then
            MSYS_NO_PATHCONV=1 wsl.exe -d Debian -- bash "/mnt/c${S#/c}/verify8/wsl/pause-baseline.sh" stop >> "$S/watchdog.log" 2>&1
            echo "$(date +%T) paused the baseline at ${free} GB free" >> "$S/watchdog.log"
            echo "ALERT memory free ${free} GB: baseline paused"
            exit 3
        fi
    fi
    disk=$(powershell.exe -NoProfile -Command "[math]::Round((Get-PSDrive C).Free/1GB,1)" 2>/dev/null | tr -d '\r' | tr ',' '.')
    if [ -n "$disk" ] && awk "BEGIN{exit !($disk < $disk_min)}"; then
        echo "ALERT disk free ${disk} GB"
        exit 2
    fi
    sleep 60
done
tail -20 "$file"
