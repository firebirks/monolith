#!/bin/sh
# Drops the page cache of the WSL VM (run as root): the memory it holds
# goes back to Windows; nothing that runs is disturbed.
sync
echo 3 > /proc/sys/vm/drop_caches
free -m | head -2
