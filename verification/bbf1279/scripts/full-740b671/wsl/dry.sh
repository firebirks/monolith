cd ~/p7
done_ids=$(grep -oE '^[A-Z]+[0-9]+ +(caught|survived)' ~/final7-mutation-phase4.log | awk '{print $1}')
python3 - "$done_ids" <<'P'
import sys
sys.path.insert(0, "mutation")
import faults
done = set(sys.argv[1].split())
left=[f["id"] for f in faults.PHASE4 if f["id"] not in done]
print(len(faults.PHASE4), len(done), len(left), " ".join(left[:12]))
P
nproc; free -m | sed -n 2p
