#!/bin/bash
# Checks the replay scripts without building or testing anything: syntax,
# the I/O sampler for a few seconds, and the line-by-line capture of
# repro-gatec.py on small commands (an ordinary end, and a time limit
# passed, as cargo_test and subprocess.run meet them).
set -u
here=$(dirname "$0")
for f in "$here"/repro-gatec.sh "$here"/repro-gatec.py "$here"/iosample.py "$here"/../diag-gatec.ps1 "$here"/pause-baseline.sh; do
    grep -q $'\r' "$f" && echo "CR in $f"
done
bash -n "$here/repro-gatec.sh" && bash -n "$here/pause-baseline.sh" && echo "bash syntax ok"
python3 -m py_compile "$here/repro-gatec.py" "$here/iosample.py" && echo "python syntax ok"
tmp=$(mktemp -d)
python3 -u "$here/iosample.py" "$tmp/io.log" & pid=$!
sleep 4; kill $pid; wait $pid 2>/dev/null
echo "== sampler"; cat "$tmp/io.log" | cut -c1-400
python3 - "$here/repro-gatec.py" "$tmp" <<'EOF'
import json, os, subprocess, sys, threading, time
text = open(sys.argv[1]).read()
part = text[text.index("# The runner's Popen"):text.index("def unmodified_run")]
out_dir = sys.argv[2]
def stamp(when=None):
    when = time.time() if when is None else when
    return time.strftime("%H:%M:%S", time.localtime(when)) + f".{int(when * 1000) % 1000:03d}"
exec(part)
subprocess.Popen = Tee
label[0] = "plain"
r = subprocess.run(["sh", "-c", "echo one; echo two >&2; sleep 1; echo three"], capture_output=True,
                   text=True, encoding="utf-8", errors="replace", timeout=10)
assert r.returncode == 0 and r.stdout == "one\nthree\n" and r.stderr == "two\n", r
label[0] = "limit"
p = subprocess.Popen(["sh", "-c", "echo start; sleep 5; echo late"], stdout=subprocess.PIPE,
                     stderr=subprocess.PIPE, text=True, start_new_session=True)
try:
    p.communicate(timeout=1)
    raise SystemExit("no timeout")
except subprocess.TimeoutExpired:
    os.killpg(p.pid, 9)
    out, err = p.communicate(timeout=60)
assert out == "start\n" and p.returncode == -9, (out, p.returncode)
label[0] = "limitrun"
try:
    subprocess.run(["sh", "-c", "echo x; sleep 5"], capture_output=True, text=True, timeout=1)
    raise SystemExit("no timeout")
except subprocess.TimeoutExpired:
    pass
print("capture ok:", sorted(os.listdir(out_dir)))
print(open(os.path.join(out_dir, "01-plain.stamped")).read(), end="")
EOF
rm -rf "$tmp"
