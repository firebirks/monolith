#!/bin/bash
cat ~/final8-out/package/listing.txt
grep -n '6974206' ~/final8-out/package.log ~/final8-out/*.txt 2>/dev/null | head -3
