#!/bin/bash
# Run as root: gives the page cache of the VM back, so that WSL returns
# that memory to Windows. Does not touch the memory of any process.
sync
echo 3 > /proc/sys/vm/drop_caches
echo 1 > /proc/sys/vm/compact_memory
