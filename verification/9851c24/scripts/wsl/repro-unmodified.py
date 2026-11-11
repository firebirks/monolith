"""Runs again, up to <count> times, the unmodified check that failed on
tree1 in gate C: the runner's own cargo_test (mutation/run.py) with the
same crates (those of the monolith-identity tests and of the 96 faults),
the same features, environment, process session and bound, on the copy
tree1. No fault is applied and no source is written. Keeps the whole
stdout and stderr of each run, its exit code, duration, the test binaries
it ran with their SHA-256, and whether anything in the copy's build was
made again. Stops at the first run that does not pass.
Usage: repro-unmodified.py <ids file> <count> <out dir>"""
import hashlib
import json
import os
import re
import subprocess
import sys
import time

ids_file, count, out_dir = sys.argv[1], int(sys.argv[2]), sys.argv[3]
sys.path.insert(0, os.path.expanduser("~/p8/mutation"))
import faults  # noqa: E402
import run  # noqa: E402

os.makedirs(out_dir, exist_ok=True)
summary = open(os.path.join(out_dir, "summary.txt"), "a", encoding="utf-8")


def note(text):
    line = f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {text}"
    print(line, flush=True)
    summary.write(line + "\n")
    summary.flush()


def sha(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def conditions():
    fields = dict(line.split(":", 1) for line in open("/proc/meminfo"))
    load = " ".join(open("/proc/loadavg").read().split()[:3])
    return f"MemAvailable {fields['MemAvailable'].strip()}, load {load}"


ids = set(open(ids_file).read().split())
everything = (faults.PHASE1 + faults.PHASE2 + faults.PHASE3 + faults.PHASE4 + faults.PHASE5
              + faults.PHASE6 + faults.PHASE7 + faults.PHASE8)
selected = [item for item in everything if item["id"] in ids]
assert len(selected) == 96, len(selected)
unmodified = sorted(set(faults.TESTS["monolith-identity"]) | {c for item in selected for c in item["crates"]})
tree = os.path.join(run.WORK, "tree1")
commit = subprocess.run(["git", "-C", run.REPO, "rev-parse", "HEAD"],
                        capture_output=True, text=True).stdout.strip()
marker = open(os.path.join(tree, ".mutation-commit")).read().strip()
note(f"HEAD {commit}, tree1 marker {marker}, crates {' '.join(unmodified)}, bound {run.UNMODIFIED_TIMEOUT}s")
assert commit == marker == "9851c24b9814092c2a50efdc6da51d99e0633460"
deps = os.path.join(tree, "target", "debug", "deps")


def executables():
    found = {}
    for name in os.listdir(deps):
        path = os.path.join(deps, name)
        if os.path.isfile(path) and os.access(path, os.X_OK) and "." not in name:
            st = os.stat(path)
            found[name] = (st.st_mtime_ns, st.st_size)
    return found


def regressions():
    return sorted(os.path.relpath(os.path.join(d, f), tree)
                  for d, _, fs in os.walk(os.path.join(tree, "crates"))
                  for f in fs if f.endswith(".proptest-regressions"))


before = executables()
note(f"executables in tree1 deps: {len(before)}; proptest regression files: {regressions()}")

captured = {}


class Tee(subprocess.Popen):
    """The runner's Popen, keeping what it was given and what it read."""

    def __init__(self, args, *rest, **kw):
        captured["command"] = list(args)
        captured["cwd"] = kw.get("cwd")
        captured["env"] = {k: v for k, v in (kw.get("env") or {}).items()
                           if k.startswith(("CARGO", "RUST")) or k in ("PATH", "HOME")}
        captured["session"] = kw.get("start_new_session")
        super().__init__(args, *rest, **kw)

    def communicate(self, *a, **kw):
        out, err = super().communicate(*a, **kw)
        captured["out"], captured["err"] = out, err
        return out, err


subprocess.Popen = Tee
for attempt in range(1, count + 1):
    captured.clear()
    note(f"run {attempt} starts; {conditions()}")
    started = time.time()
    code, output, timed_out = run.cargo_test(tree, unmodified, run.UNMODIFIED_TIMEOUT)
    seconds = round(time.time() - started)
    out, err = captured.get("out", ""), captured.get("err", "")
    if not out and not err:
        out = output
    for kind, text in (("stdout", out), ("stderr", err)):
        with open(os.path.join(out_dir, f"run{attempt}.{kind}"), "w", encoding="utf-8") as handle:
            handle.write(text)
    if attempt == 1:
        note(f"command {' '.join(captured.get('command', []))}; cwd {captured.get('cwd')}; "
             f"new session {captured.get('session')}")
        note(f"environment given to cargo {json.dumps(captured.get('env', {}), sort_keys=True)}")
    ran = re.findall(r"^\s+Running \S+ \((target/debug/deps/[^)]+)\)", err, re.M)
    hashes = {os.path.basename(p): sha(os.path.join(tree, p)) for p in ran}
    with open(os.path.join(out_dir, f"run{attempt}.binaries.json"), "w", encoding="utf-8") as handle:
        json.dump(hashes, handle, indent=1, sort_keys=True)
    after = executables()
    changed = sorted(n for n in after if before.get(n) != after[n])
    results = re.findall(r"^test result: (\w+)\. (\d+) passed; (\d+) failed", out, re.M)
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", out, re.M)))
    passed_total = sum(int(r[1]) for r in results)
    failed_total = sum(int(r[2]) for r in results)
    tr = next((h[:16] for n, h in hashes.items() if n.startswith("transfer_restarts-")), None)
    note(f"run {attempt}: exit {code}, timed out {timed_out}, {seconds}s, {len(ran)} binaries run "
         f"(transfer_restarts {tr}), {len(results)} result lines, {passed_total} passed, "
         f"{failed_total} failed {failed}; rebuilt since the start: {changed}; "
         f"regression files {regressions()}; {conditions()}")
    if code != 0 or timed_out:
        note(f"run {attempt} did not pass: stop. stdout and stderr in {out_dir}/run{attempt}.*")
        sys.exit(1)
note(f"all {count} runs passed")
