date; cat ~/targeted-mutation.txt
for f in ~/targeted-mutation-phase*.log; do echo "== $f"; head -1 $f; grep -cE "^[A-Z]+[0-9]+ +(caught|survived|invalid)" $f; grep -E "survived|invalid|timeout|UNEXPECTED|MR36" $f | sort -u; tail -1 $f; done
free -m | sed -n 2,3p
