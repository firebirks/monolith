"""Read only: the checks of gate C3 beyond validate-c2.py.

1. Each fault caught by the timeout: the binary running when the run was
   ended, the tests over 60 s, and how long that binary takes in the
   unmodified run of the same day: a timeout that is a hang, not a slow
   pass.
2. The proptest regression files the faults left in the copies
   (crates/monolith-protocol/tests/properties.proptest-regressions): which
   fault ran in which copy and when, which faults ran in a copy after its
   file was written, and of those, which were caught by `properties`, and
   whether by it alone.
Usage: c3-extra.py <merged dir>"""
import datetime
import json
import os
import re
import sys

merged = sys.argv[1]
logs = os.path.join(merged, "logs")
data = json.load(open(os.path.join(merged, "results-C3-merged.json"), encoding="utf-8"))
results = {r["id"]: r for r in data["results"]}
WORK = "<repo>/mutation/work"
unmodified = {"2026-11-10": f"{WORK}/logs/all-c90d5e5-20261110-151711",
              "2026-11-11": f"{WORK}/logs/all-c90d5e5-20261111-082454"}


def binaries_and_times(stdout_path, stderr_path):
    """[(binary, seconds)] in the order cargo ran them."""
    stderr = open(stderr_path, encoding="utf-8", errors="replace").read().splitlines()
    stdout = open(stdout_path, encoding="utf-8", errors="replace").read().splitlines()
    started = []
    for line in stderr:
        running = re.match(r"^\s+Running (.+) \((\S+)\)$", line)
        doc = re.match(r"^\s+Doc-tests (\S+)$", line)
        if running:
            started.append(os.path.basename(running.group(2)).rsplit("-", 1)[0]
                           + (" (unit)" if running.group(1).startswith("unittests") else ""))
        elif doc:
            started.append(f"doc-tests {doc.group(1)}")
    times = [float(m) for m in re.findall(r"^test result: .*finished in ([\d.]+)s$", "\n".join(stdout), re.M)]
    return started, times


def tree_of(fid):
    stderr = open(os.path.join(logs, f"{fid}.stderr"), encoding="utf-8", errors="replace").read()
    trees = set(re.findall(r"/mutation/work/(tree\d)/", stderr))
    return trees.pop() if len(trees) == 1 else f"? {sorted(trees)}"


def ended(fid):
    return datetime.datetime.fromtimestamp(max(
        os.path.getmtime(os.path.realpath(os.path.join(logs, f"{fid}.{k}"))) for k in ("stdout", "stderr")))


print("== 1. the faults caught by the timeout")
normal = {}
for day, directory in unmodified.items():
    for tree in ("tree0", "tree1"):
        started, times = binaries_and_times(f"{directory}/{tree}-unmodified.stdout",
                                            f"{directory}/{tree}-unmodified.stderr")
        for binary, seconds in zip(started, times):
            normal.setdefault(binary, []).append(seconds)
for fid, r in results.items():
    if "(timeout)" not in r["failed"]:
        continue
    started, times = binaries_and_times(os.path.join(logs, f"{fid}.stdout"), os.path.join(logs, f"{fid}.stderr"))
    stdout = open(os.path.join(logs, f"{fid}.stdout"), encoding="utf-8", errors="replace").read()
    slow = re.findall(r"^test (\S+) has been running for over 60 seconds$", stdout, re.M)
    last = started[-1] if started else None
    finished_last = len(times) == len(started)
    print(f"{fid} ({r['run']}, {r['seconds']}s, {tree_of(fid)}): {len(started)} binaries started, "
          f"{len(times)} finished; running when ended: {last if not finished_last else 'none (all finished)'}; "
          f"that binary unmodified: {normal.get(last, ['?'])} s")
    print(f"    tests over 60 s in the binary running at the end: "
          f"{slow[-3:] if slow else 'none'}; named failures: {[n for n in r['failed'] if not n.startswith('(')]}")

print("\n== 2. the proptest regression files in the copies")
for tree in ("tree0", "tree1"):
    path = f"{WORK}/{tree}/crates/monolith-protocol/tests/properties.proptest-regressions"
    written = datetime.datetime.fromtimestamp(os.path.getmtime(path))
    seeds = [line for line in open(path) if line.startswith("cc ")]
    print(f"{tree}: written {written:%Y-%m-%d %H:%M:%S}, {len(seeds)} seeds")
    for line in seeds:
        print("    " + line.strip()[:160])
    runs = []
    for fid, r in results.items():
        if tree_of(fid) != tree:
            continue
        end = ended(fid)
        start = end - datetime.timedelta(seconds=r["seconds"])
        runs.append((start, end, fid, r))
    runs.sort()
    writer = [fid for start, end, fid, r in runs if start <= written <= end + datetime.timedelta(seconds=5)]
    print(f"  running in {tree} when it was written: {writer}")
    after = [(fid, r) for start, end, fid, r in runs if start > written]
    print(f"  faults started in {tree} after: {len(after)}")
    for fid, r in after:
        stdout = open(os.path.join(logs, f"{fid}.stdout"), encoding="utf-8", errors="replace").read()
        started, _ = binaries_and_times(os.path.join(logs, f"{fid}.stdout"), os.path.join(logs, f"{fid}.stderr"))
        if "properties" not in started:
            continue
        failing = {}
        index = -1
        for line in stdout.splitlines():
            if re.match(r"^running \d+ tests?$", line):
                index += 1
            name = re.match(r"^test (.+) \.\.\. FAILED$", line)
            if name:
                failing.setdefault(started[index] if 0 <= index < len(started) else "?", []).append(name.group(1))
        verdict = ("properties alone" if set(failing) == {"properties"} else
                   "properties and others" if "properties" in failing else "not by properties")
        print(f"    {fid}: ran properties; {verdict}; failing {failing or r['failed']}")
