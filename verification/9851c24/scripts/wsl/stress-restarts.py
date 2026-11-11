"""Stress of transfer_restarts under CPU contention, authorized by the
owner on 2026-11-10: the test binary gate C ran and failed (tree1 of the
runner copies, built from 9851c24, sha256 4f80612a...), run as it is,
without building anything and with no test parameter set, as PROCESSES
processes at once, for up to ROUNDS rounds. A round where a process does
not pass is let end (the others keep their whole logs) and is the last.

For each process: stdout and stderr in files of their own, written as
they come, the exit code, the duration, the CPU time (user, system) and
the peak memory. The I/O sampler runs beside the rounds.

Usage: stress-restarts.py <out dir> (a directory that does not exist)"""
import glob
import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import time

TREE = os.path.expanduser("~/p8/mutation/work/tree1")
BINARY = os.path.join(TREE, "target/debug/deps/transfer_restarts-9a2146a26bc681d6")
SHA256 = "4f80612a0854f680390147ad3faeb6bb832178e0675f4bdc4abfeffcf8a8c9f7"
CRATE = os.path.join(TREE, "crates/monolith-core")
COMMIT = "9851c24"
PROCESSES = 4
ROUNDS = 10
# A process that has not ended in this many seconds is a hang: alone the
# binary takes about 160 s.
BOUND = 2400
HERE = os.path.dirname(os.path.abspath(__file__))

out_dir = sys.argv[1]
os.makedirs(out_dir)
summary = open(os.path.join(out_dir, "summary.txt"), "a", encoding="utf-8")


def note(text):
    line = f"{time.strftime('%Y-%m-%dT%H:%M:%S')} {text}"
    print(line, flush=True)
    summary.write(line + "\n")
    summary.flush()


def sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for block in iter(lambda: handle.read(1 << 20), b""):
            digest.update(block)
    return digest.hexdigest()


def regression_files():
    return sorted(set(glob.glob(os.path.join(TREE, "crates/**/*.proptest-regressions"), recursive=True)))


def available_mb():
    for line in open("/proc/meminfo"):
        if line.startswith("MemAvailable:"):
            return int(line.split()[1]) // 1024
    return 0


def threads_of(pid):
    """State and kernel wait channel of each thread of a process, for a
    process that reached the bound."""
    rows = []
    for task in sorted(glob.glob(f"/proc/{pid}/task/*")):
        try:
            stat = open(os.path.join(task, "stat")).read()
            name = stat[stat.index("(") + 1:stat.rindex(")")]
            state = stat[stat.rindex(")") + 2]
            wchan = open(os.path.join(task, "wchan")).read().strip()
            rows.append(f"{os.path.basename(task)} {state} {name} {wchan}")
        except OSError:
            pass
    return rows


def digest_of(stdout_path):
    """What the stdout of a run tells: the result line, the tests that
    failed and those that ran over 60 seconds."""
    result, failed, slow = None, [], []
    with open(stdout_path, encoding="utf-8", errors="replace") as handle:
        for line in handle:
            line = line.rstrip("\n")
            if line.startswith("test result:"):
                result = line
            name = re.match(r"^test (.+) \.\.\. FAILED$", line)
            if name:
                failed.append(name.group(1))
            name = re.match(r"^test (.+) has been running for over 60 seconds$", line)
            if name:
                slow.append(name.group(1))
    return result, failed, slow


# The conditions: the copy is that of the candidate, nothing else runs,
# no test parameter comes from the environment.
assert open(os.path.join(TREE, ".mutation-commit")).read().strip().startswith(COMMIT)
assert sha256(BINARY) == SHA256, "the binary is not the one gate C ran"
assert not regression_files(), regression_files()
others = subprocess.run(["pgrep", "-af", r"cargo|rustc|mutation/run\.py|/target/debug/deps/"],
                        capture_output=True, text=True).stdout
others = [line for line in others.splitlines() if "pgrep" not in line]
assert not others, others
env = dict(os.environ, CARGO_TERM_COLOR="never")
unwanted = [key for key in env if key.startswith(("PROPTEST", "RUST_TEST", "RUST_BACKTRACE", "RUST_LOG"))]
assert not unwanted, unwanted
# What cargo test gives the binary: its crate directory as the working
# directory, and the manifest variables.
env.update(
    CARGO_MANIFEST_DIR=CRATE,
    CARGO_MANIFEST_PATH=os.path.join(CRATE, "Cargo.toml"),
    CARGO_PKG_NAME="monolith-core",
)
with open(os.path.join(out_dir, "conditions.json"), "w") as handle:
    json.dump({"binary": BINARY, "sha256": SHA256, "cwd": CRATE, "processes": PROCESSES,
               "rounds": ROUNDS, "bound": BOUND, "argv": [BINARY],
               "env": {k: v for k, v in env.items() if k.startswith(("CARGO", "RUST", "PROPTEST", "PATH", "HOME"))},
               "nproc": os.cpu_count(), "mem_available_mb": available_mb()}, handle, indent=1)

sampler = subprocess.Popen([sys.executable, "-u", os.path.join(HERE, "iosample.py"),
                            os.path.join(out_dir, "io.log")])
note(f"stress started: {PROCESSES} processes of {os.path.basename(BINARY)} (sha256 {SHA256[:16]}), "
     f"up to {ROUNDS} rounds, bound {BOUND}s; MemAvailable {available_mb()} MB")
failed_round = None
try:
    for number in range(1, ROUNDS + 1):
        round_dir = os.path.join(out_dir, f"round{number:02d}")
        os.makedirs(round_dir)
        waited = 0
        while available_mb() < 3000 and waited < 600:
            time.sleep(10)
            waited += 10
        if available_mb() < 3000:
            note(f"round {number}: MemAvailable {available_mb()} MB after {waited}s: stop")
            failed_round = number
            break
        runs = []
        for index in range(1, PROCESSES + 1):
            stem = os.path.join(round_dir, f"proc{index}")
            stdout = open(stem + ".stdout", "wb")
            stderr = open(stem + ".stderr", "wb")
            started = time.time()
            process = subprocess.Popen([BINARY], cwd=CRATE, env=env, stdout=stdout, stderr=stderr,
                                       start_new_session=True)
            stdout.close()
            stderr.close()
            runs.append({"index": index, "pid": process.pid, "process": process, "stem": stem,
                         "started": started, "ended": None})
        left = {run["pid"]: run for run in runs}
        while left:
            pid, status, usage = os.wait4(-1, os.WNOHANG)
            if pid in left:
                run = left.pop(pid)
                run["process"].returncode = os.waitstatus_to_exitcode(status)
                run.update(ended=time.time(), exit=os.waitstatus_to_exitcode(status),
                           user=usage.ru_utime, system=usage.ru_stime, maxrss_kb=usage.ru_maxrss)
                continue
            if pid == 0:
                now = time.time()
                for run in list(left.values()):
                    if now - run["started"] > BOUND and not run.get("timed_out"):
                        run["timed_out"] = True
                        run["threads"] = threads_of(run["pid"])
                        os.killpg(run["pid"], signal.SIGKILL)
                time.sleep(0.2)
        exits = []
        for run in runs:
            result, failed, slow = digest_of(run["stem"] + ".stdout")
            seconds = run["ended"] - run["started"]
            record = {
                "round": number, "index": run["index"], "pid": run["pid"],
                "started": time.strftime("%H:%M:%S", time.localtime(run["started"])),
                "ended": time.strftime("%H:%M:%S", time.localtime(run["ended"])),
                "seconds": round(seconds, 1), "exit": run["exit"], "timed_out": bool(run.get("timed_out")),
                "user_s": round(run["user"], 1), "system_s": round(run["system"], 1),
                "cpu_percent": round(100 * (run["user"] + run["system"]) / seconds),
                "maxrss_mb": round(run["maxrss_kb"] / 1024), "result": result,
                "failed": failed, "over_60s": slow, "threads_at_bound": run.get("threads", []),
            }
            with open(run["stem"] + ".json", "w") as handle:
                json.dump(record, handle, indent=1)
            exits.append(record)
        bad = [r for r in exits if r["exit"] != 0 or r["timed_out"]]
        regressions = regression_files()
        intact = sha256(BINARY) == SHA256
        note(f"round {number}: " + "; ".join(
            f"p{r['index']} exit {r['exit']}{' TIMEOUT' if r['timed_out'] else ''} {r['seconds']:.0f}s "
            f"cpu {r['user_s'] + r['system_s']:.0f}s ({r['cpu_percent']}%) rss {r['maxrss_mb']}MB "
            f"{(r['result'] or 'no result line').replace('test result: ', '')[:60]}"
            for r in exits)
            + f"; MemAvailable {available_mb()} MB; binary {'unchanged' if intact else 'CHANGED'}"
            + (f"; regression files {regressions}" if regressions else ""))
        for r in bad:
            note(f"round {number} p{r['index']} did not pass: failed {r['failed']}, over 60 s {r['over_60s']}")
        if bad or regressions or not intact:
            failed_round = number
            break
finally:
    sampler.send_signal(signal.SIGTERM)
    sampler.wait()
if failed_round:
    note(f"stopped after round {failed_round}")
    sys.exit(1)
note(f"all {ROUNDS} rounds passed")
