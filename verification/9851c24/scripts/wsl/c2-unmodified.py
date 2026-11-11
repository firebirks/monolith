"""Gate C on c896861, first step (owner, 2026-11-10): the two unmodified
checks of the runner, tree0 then tree1, with the runner's own functions
and in its own order (prepare, precheck, then for each copy the
unmodified run and check_copy), and nothing else: no fault. The run of
the 96 follows only once these pass and their logs are looked at.

For each copy it writes, beside the runner's logs, the SHA-256 of every
test binary the unmodified run ran, and compares them with those of the
same copy built from 9851c24. The I/O sampler runs meanwhile.

Usage: c2-unmodified.py <ids file> <out dir>"""
import json
import os
import re
import signal
import subprocess
import sys
import time
import hashlib

NEW = "c896861b66b0296db4ab1598589d69a88954d877"
P8 = os.path.expanduser("~/p8")
HERE = os.path.dirname(os.path.abspath(__file__))
ids = open(sys.argv[1]).read().split()
out_dir = sys.argv[2]
os.makedirs(out_dir)

head = subprocess.run(["git", "-C", P8, "rev-parse", "HEAD"], capture_output=True, text=True).stdout.strip()
assert head == NEW, head
assert not subprocess.run(["git", "-C", P8, "status", "--porcelain", "--untracked-files=no"],
                          capture_output=True, text=True).stdout.strip()
sys.path.insert(0, os.path.join(P8, "mutation"))
import faults  # noqa: E402
import run  # noqa: E402

assert os.path.dirname(os.path.abspath(run.__file__)) == os.path.join(P8, "mutation")
assert hasattr(run, "failure_report"), "not the runner of c896861"


def note(text):
    print(f"{time.strftime('%H:%M:%S')} {text}", flush=True)


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def check_logs(logged):
    """What the files of an unmodified run hold: every binary cargo ran
    has its result line, and the totals."""
    stderr = open(logged + ".stderr", encoding="utf-8", errors="replace").read()
    stdout = open(logged + ".stdout", encoding="utf-8", errors="replace").read()
    ran = re.findall(r"^\s+Running (.+) \((\S+)\)$", stderr, re.M)
    doc = re.findall(r"^\s+Doc-tests (\S+)$", stderr, re.M)
    results = re.findall(r"^test result: (\w+)\. (\d+) passed; (\d+) failed; (\d+) ignored", stdout, re.M)
    return {
        "stdout_bytes": len(stdout.encode()), "stderr_bytes": len(stderr.encode()),
        "binaries_run": len(ran), "doc_tests": len(doc), "result_lines": len(results),
        "passed": sum(int(r[1]) for r in results), "failed": sum(int(r[2]) for r in results),
        "ignored": sum(int(r[3]) for r in results),
        "not_ok": [r for r in results if r[0] != "ok"],
        "finished": bool(re.search(r"^\s+Finished `test` profile", stderr, re.M)),
        "binaries": [path for _, path in ran],
    }


lock = run.lock_work()
sampler = subprocess.Popen([sys.executable, "-u", os.path.join(HERE, "iosample.py"), os.path.join(out_dir, "io.log")])
try:
    selected = [item for item in faults.PHASE1 + faults.PHASE2 + faults.PHASE3 + faults.PHASE4
                + faults.PHASE5 + faults.PHASE6 + faults.PHASE7 + faults.PHASE8 if item["id"] in set(ids)]
    assert len(selected) == 96, len(selected)
    note(f"commit {NEW}: preparing the copies")
    trees = [run.prepare(worker, NEW) for worker in (0, 1)]
    for tree in trees:
        assert open(os.path.join(tree, ".mutation-commit")).read().strip() == NEW
    problems = run.precheck(selected, trees[0])
    assert not problems, problems
    note("precheck: every pattern found once")
    unmodified = sorted(set(faults.TESTS["monolith-identity"])
                        | {crate for item in selected for crate in item["crates"]})
    note(f"unmodified run of: {' '.join(unmodified)}")
    logs = os.path.join(run.LOGS, f"unmodified-{NEW[:7]}-{time.strftime('%Y%m%d-%H%M%S')}")
    os.makedirs(logs)
    note(f"runner logs in {logs}")
    for tree in trees:
        name = os.path.basename(tree)
        logged = os.path.join(logs, f"{name}-unmodified")
        note(f"{name}: unmodified run")
        seconds = run.unmodified_run(tree, unmodified, logged)
        facts = check_logs(logged)
        binaries = {os.path.basename(path): sha256(os.path.join(tree, path)) for path in facts.pop("binaries")}
        with open(os.path.join(out_dir, f"{name}-binaries.json"), "w") as handle:
            json.dump(binaries, handle, indent=1, sort_keys=True)
        before = json.load(open(os.path.expanduser(f"~/evidence-9851c24/{name}-binaries-9851c24.json")))
        same = sorted(k for k in binaries if before.get(k) == binaries[k])
        other = sorted(k for k in binaries if k in before and before[k] != binaries[k])
        new = sorted(k for k in binaries if k not in before)
        gone = sorted(k for k in before if k not in binaries)
        note(f"{name}: passed in {seconds}s; logs {facts}")
        note(f"{name}: {len(binaries)} test binaries; against 9851c24: {len(same)} identical, "
             f"{len(other)} different {other}, {len(new)} not there before {new}, {len(gone)} gone {gone}")
        problem = run.check_copy(tree)
        if problem:
            note(f"{name}: check_copy: {problem}")
            sys.exit(3)
        note(f"{name}: check_copy passed")
    note("both unmodified checks passed")
finally:
    sampler.send_signal(signal.SIGTERM)
    sampler.wait()
    os.remove(lock)
