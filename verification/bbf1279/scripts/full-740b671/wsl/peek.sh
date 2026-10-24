date; tail -4 ~/final7-mutation.txt
for f in ~/final7-mutation-phase2.log ~/final7-mutation-phase1.log; do [ -f $f ] && { echo "== $f"; head -1 $f; grep -cE "^[A-Z]+[0-9]+ +(caught|survived)" $f; grep -E "survived|UNEXPECTED|invalid|build" $f | sort -u; tail -1 $f; }; done
ps aux | grep -c "[c]argo"
