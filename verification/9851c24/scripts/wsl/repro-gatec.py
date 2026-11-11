"""One cycle of the replay of the start of gate C, on a WSL just started
again (cold page cache), with the runner's own functions in the runner's
own order (mutation/run.py run_main):
  1. the unmodified run of tree0 (cargo_test, the crates of the 96
     faults) and the check of the copy tree0 (check_copy: its unit tests,
     with the canary X1 applied, and once X1 is restored);
  2. the sources of tree1 the runner restored during gate B (the list
     touch-tree1.txt) get a new time, as the restore gave them
     (os.utime), their content unchanged and checked against the commit;
  3. at once, the unmodified run of tree1, building and testing.
No other fault is applied and no source content is written. For every
command: its whole stdout and stderr, also line by line with the time
each line came, exit code and duration; for the unmodified runs the
SHA-256 of every test binary run and what each step built; all along,
the I/O and the load sampled every second (iosample.py). Exits 1 at the
first step that does not pass.
Usage: repro-gatec.py <ids file> <cycle> <out dir>"""
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time

ids_file, cycle, out_dir = sys.argv[1], sys.argv[2], sys.argv[3]
HERE = os.path.dirname(os.path.abspath(__file__))
DIAG = os.path.expanduser("~/diag-gatec")
sys.path.insert(0, os.path.expanduser("~/p8/mutation"))
import faults  # noqa: E402
import run  # noqa: E402

COMMIT = "9851c24b9814092c2a50efdc6da51d99e0633460"
os.makedirs(out_dir, exist_ok=True)
summary = open(os.path.join(DIAG, "summary.txt"), "a", encoding="utf-8")
steps = open(os.path.join(out_dir, "steps.txt"), "a", encoding="utf-8")


def stamp(when=None):
    when = time.time() if when is None else when
    return time.strftime("%H:%M:%S", time.localtime(when)) + f".{int(when * 1000) % 1000:03d}"


def note(text):
    line = f"{time.strftime('%Y-%m-%dT%H:%M:%S')} cycle {cycle} {text}"
    print(line, flush=True)
    for handle in (summary, steps):
        handle.write(line + "\n")
        handle.flush()


def sha(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def conditions():
    fields = {}
    for line in open("/proc/meminfo"):
        name, value = line.split(":", 1)
        fields[name] = int(value.split()[0]) // 1024
    load = " ".join(open("/proc/loadavg").read().split()[:3])
    uptime = open("/proc/uptime").read().split()[0]
    return (f"uptime {uptime}s, MemAvailable {fields['MemAvailable']}MB, Cached {fields['Cached']}MB, "
            f"Dirty {fields['Dirty']}MB, load {load}")


def outputs(tree):
    deps = os.path.join(tree, "target", "debug", "deps")
    found = {}
    for name in os.listdir(deps):
        if name.endswith(".d"):
            continue
        st = os.stat(os.path.join(deps, name))
        found[name] = (st.st_mtime_ns, st.st_size)
    return found


def regressions(tree):
    return sorted(os.path.relpath(os.path.join(d, f), tree)
                  for d, _, fs in os.walk(os.path.join(tree, "crates"))
                  for f in fs if f.endswith(".proptest-regressions"))


# The runner's Popen, keeping for every command what it was given, and
# what it wrote with the time each line came. Its pipes are drained as
# they fill, as communicate drains them; the runner gets the same text.
Popen = subprocess.Popen
label = ["setup"]
counter = [0]


class Tee(Popen):
    def __init__(self, args, *rest, **kw):
        super().__init__(args, *rest, **kw)
        counter[0] += 1
        self.tag = os.path.join(out_dir, f"{counter[0]:02d}-{label[0]}")
        self.lines = {"stdout": [], "stderr": []}
        self.lock = threading.Lock()
        self.stamped = open(self.tag + ".stamped", "w", encoding="utf-8")
        with open(self.tag + ".command.json", "w", encoding="utf-8") as handle:
            json.dump({
                "command": list(args), "cwd": kw.get("cwd"), "started": stamp(),
                "new session": kw.get("start_new_session"),
                "env": {k: v for k, v in (kw.get("env") or {}).items()
                        if k.startswith(("CARGO", "RUST")) or k in ("PATH", "HOME")},
            }, handle, indent=1)
        self.readers = []
        for kind in ("stdout", "stderr"):
            stream = getattr(self, kind)
            if stream is not None:
                reader = threading.Thread(target=self.drain, args=(kind, stream), daemon=True)
                reader.start()
                self.readers.append(reader)

    def drain(self, kind, stream):
        for line in stream:
            when = time.time()
            self.lines[kind].append(line)
            with self.lock:
                self.stamped.write(f"{stamp(when)} {'O' if kind == 'stdout' else 'E'} "
                                   f"{line if line.endswith(chr(10)) else line + chr(10)}")
                self.stamped.flush()

    def communicate(self, input=None, timeout=None):
        assert input is None
        deadline = None if timeout is None else time.monotonic() + timeout
        self.wait(timeout=timeout)
        for reader in self.readers:
            reader.join(None if deadline is None else max(0, deadline - time.monotonic()))
            if reader.is_alive():
                raise subprocess.TimeoutExpired(self.args, timeout)
        out, err = "".join(self.lines["stdout"]), "".join(self.lines["stderr"])
        for kind, text in (("stdout", out), ("stderr", err)):
            with open(f"{self.tag}.{kind}", "w", encoding="utf-8") as handle:
                handle.write(text)
        with open(self.tag + ".exit", "w", encoding="utf-8") as handle:
            handle.write(f"{self.returncode} {stamp()}\n")
        self.stamped.close()
        self.last = (out, err)
        return out, err


def unmodified_run(tree, name):
    """The runner's unmodified run of `tree`, as run_main makes it."""
    label[0] = name
    built_before = outputs(tree)
    note(f"{name} starts; {conditions()}")
    started = time.time()
    code, output, timed_out = run.cargo_test(tree, unmodified, run.UNMODIFIED_TIMEOUT)
    seconds = round(time.time() - started)
    ran =re.findall(r"^\s+Running \S+ \((target/debug/deps/[^)]+)\)", output, re.M)
    hashes = {os.path.basename(p): sha(os.path.join(tree, p)) for p in ran}
    with open(os.path.join(out_dir, f"{name}.binaries.json"), "w", encoding="utf-8") as handle:
        json.dump(hashes, handle, indent=1, sort_keys=True)
    built_after = outputs(tree)
    built = sorted(n for n in built_after if built_before.get(n) != built_after[n])
    with open(os.path.join(out_dir, f"{name}.built.txt"), "w", encoding="utf-8") as handle:
        handle.write("".join(f"{n}\n" for n in built))
    results = re.findall(r"^test result: (\w+)\. (\d+) passed; (\d+) failed", output, re.M)
    failed = sorted(set(re.findall(r"^test (\S+) \.\.\. FAILED", output, re.M)))
    restarts = next((f"{n} {h[:16]}" for n, h in hashes.items() if n.startswith("transfer_restarts-")), None)
    finished = re.search(r"Finished `test` profile .* in (\S+)", output)
    note(f"{name}: exit {code}, timed out {timed_out}, {seconds}s, build {finished.group(1) if finished else '?'}, "
         f"{len(built)} build outputs written, {len(ran)} binaries run (transfer_restarts {restarts}), "
         f"{len(results)} result lines, {sum(int(r[1]) for r in results)} passed, "
         f"{sum(int(r[2]) for r in results)} failed {failed}; regression files {regressions(tree)}; {conditions()}")
    return code == 0 and not timed_out, built


def same_as_commit(path):
    with open(os.path.join(trees[1], path), "rb") as handle:
        return handle.read() == blobs[path]


ids = set(open(ids_file).read().split())
everything = (faults.PHASE1 + faults.PHASE2 + faults.PHASE3 + faults.PHASE4 + faults.PHASE5
              + faults.PHASE6 + faults.PHASE7 + faults.PHASE8)
selected = [item for item in everything if item["id"] in ids]
assert len(selected) == 96, len(selected)
unmodified = sorted(set(faults.TESTS["monolith-identity"]) | {c for item in selected for c in item["crates"]})
touch = open(os.path.join(DIAG, "touch-tree1.txt")).read().split()
blobs = {path: subprocess.run(["git", "-C", run.REPO, "show", f"{COMMIT}:{path}"],
                              capture_output=True, check=True).stdout for path in touch}
commit =subprocess.run(["git", "-C", run.REPO, "rev-parse", "HEAD"],
                        capture_output=True, text=True, check=True).stdout.strip()
for worker in (0, 1):
    marker = run.read(os.path.join(run.WORK, f"tree{worker}", ".mutation-commit")).strip()
    assert commit == marker == COMMIT, (worker, commit, marker)
others = subprocess.run(["pgrep", "-af", "mutation/run.py|cargo"], capture_output=True, text=True).stdout
assert not others.strip(), others
# As run_main: the copies are kept (the marker names the commit) and the
# patterns of the selected faults are each found once.
trees = [run.prepare(worker, COMMIT) for worker in (0, 1)]
assert not run.precheck(selected, trees[0])
note(f"starts: HEAD {commit}; crates {' '.join(unmodified)}; bound {run.UNMODIFIED_TIMEOUT}s; "
     f"{len(touch)} sources of tree1 to touch; {conditions()}")

sampler = Popen([sys.executable, "-u", os.path.join(HERE, "iosample.py"), os.path.join(out_dir, "io.log")])
subprocess.Popen = Tee
code = 0
try:
    passed, _ = unmodified_run(trees[0], "tree0-unmodified")
    if not passed:
        note("tree0-unmodified did not pass: stop")
        code = 1
    else:
        label[0] = "tree0-check"
        note(f"tree0-check starts; {conditions()}")
        started = time.time()
        problem = run.check_copy(trees[0])
        note(f"tree0-check: {problem or 'passed'} ({round(time.time() - started)}s; "
             f"regression files {regressions(trees[0])})")
        if problem:
            note("tree0-check did not pass: stop")
            code = 1
    if code == 0:
        label[0] = "tree1-touch"
        changed = []
        for path in touch:
            full = os.path.join(trees[1], path)
            before = os.stat(full).st_mtime_ns
            assert same_as_commit(path), path
            os.utime(full, None)
            assert same_as_commit(path), path
            changed.append(f"{path} {stamp(before / 1e9)} -> {stamp(os.stat(full).st_mtime_ns / 1e9)}")
        with open(os.path.join(out_dir, "tree1-touch.txt"), "w", encoding="utf-8") as handle:
            handle.write("".join(f"{line}\n" for line in changed))
        note(f"tree1-touch: {len(changed)} sources given a new time, content as the commit's")
        passed, built = unmodified_run(trees[1], "tree1-unmodified")
        gate_c = {line.split()[3] for line in open(os.path.join(DIAG, "gatec-built-tree1.txt"))}
        missing = sorted(gate_c - set(built))
        extra = sorted(set(built) - gate_c)
        note(f"tree1 build outputs against gate C: {len(gate_c)} written by gate C, {len(built)} now; "
             f"of gate C's not written now: {missing}; written now, not by gate C: {len(extra)} {extra}")
        if not passed:
            note("tree1-unmodified did not pass: stop")
            code = 1
finally:
    subprocess.Popen = Popen
    sampler.send_signal(signal.SIGTERM)
    sampler.wait()
note(f"ends with {code}")
sys.exit(code)
