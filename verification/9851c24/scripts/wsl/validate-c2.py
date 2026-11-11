"""Read only: the validation of gate C on c896861 the owner asked for
(2026-11-10). "96/96 caught" is not enough: for each fault, from the
runner's results and the whole log of its run,

- the verdict against the expectation; invalid, survived, unexpected;
- the test targets cargo says failed (`error: N targets failed`), and
  the failing tests of each test binary (stdout blocks matched in order
  with the Running and Doc-tests lines of stderr);
- the faults that reached the timeout, and those caught by the timeout
  alone, with the tests that ran over 60 s and the binary that was
  running when the run was ended;
- every fault where transfer_restarts failed, with its failures, and
  above all those where it is the only target that failed: a catch that
  rests on transfer_restarts alone is looked at by hand before it counts;
- the earlier evidence of the fault (740b671, ba12472, bbf1279): the
  tests that caught it then, and which of them fail now.

Usage: validate-c2.py <results json> <logs dir> <evidence dir> <ids file>"""
import json
import os
import re
import sys

results_path, logs, evidence_dir, ids_path = sys.argv[1:5]
data = json.load(open(results_path, encoding="utf-8"))
results = {r["id"]: r for r in data["results"]}
ids = open(ids_path).read().split()
print(f"commit {data['commit']}, unmodified {data['unmodified_seconds']}s, timeout {data['timeout']}s, "
      f"{len(results)} results")
problems = []
if sorted(results) != sorted(ids) or len(results) != len(ids) or len(set(ids)) != len(ids):
    problems.append(f"results {len(results)} do not match the {len(ids)} ids: "
                    f"missing {sorted(set(ids) - set(results))}, extra {sorted(set(results) - set(ids))}")

# Gate C3 (c90d5e5) also reruns MR26 (gate B on 9851c24) and FT29 (the
# Phase 8 list, part 2, on 9851c24): their latest evidence is from there.
earlier = {}
for name in sorted(os.listdir(evidence_dir)):
    match = (re.match(r"results-phase[1-7]-(740b671|ba12472|bbf1279)\.json$", name)
             or re.match(r"results-(?:B|phase8)-(9851c24)(?:-part2)?\.json$", name))
    if match:
        order = ["740b671", "ba12472", "bbf1279", "9851c24"].index(match.group(1))
        for r in json.load(open(os.path.join(evidence_dir, name), encoding="utf-8"))["results"]:
            if r["id"] not in earlier or earlier[r["id"]][0] < order:
                earlier[r["id"]] = (order, match.group(1), r)


def blocks(logged):
    """{binary: [failing tests]} of one run, the targets cargo said failed,
    the tests over 60 s and the last binary started."""
    stderr = open(logged + ".stderr", encoding="utf-8", errors="replace").read().splitlines()
    stdout = open(logged + ".stdout", encoding="utf-8", errors="replace").read().splitlines()
    started = []
    for line in stderr:
        running = re.match(r"^\s+Running (.+) \((\S+)\)$", line)
        doc = re.match(r"^\s+Doc-tests (\S+)$", line)
        if running:
            started.append(os.path.basename(running.group(2)).rsplit("-", 1)[0]
                           + (" (unit)" if running.group(1).startswith("unittests") else ""))
        elif doc:
            started.append(f"doc-tests {doc.group(1)}")
    failing, slow, index, failures, current = {}, [], -1, {}, None
    for line in stdout:
        if re.match(r"^running \d+ tests?$", line):
            index += 1
        name = re.match(r"^test (.+) \.\.\. FAILED$", line)
        binary = started[index] if 0 <= index < len(started) else f"block {index}"
        if name:
            failing.setdefault(binary, []).append(name.group(1))
        over = re.match(r"^test (.+) has been running for over 60 seconds$", line)
        if over:
            slow.append(f"{binary}: {over.group(1)}")
        # The panic lines of each failing test, from its section.
        header = re.match(r"^---- (.+) stdout ----$", line)
        if header:
            current = (binary, header.group(1))
            failures[current] = []
        elif current and (line.startswith("failures:") or line.startswith("test result:")):
            current = None
        elif current and len(failures[current]) < 6 and line.strip():
            failures[current].append(line.strip()[:200])
    targets = [line.strip() for line in stderr if line.startswith("    `-p ")]
    return failing, targets, slow, (started[-1] if started else None), len(started), index + 1, failures


rows = []
for fid in ids:
    r = results.get(fid)
    if r is None:
        continue
    logged = os.path.join(logs, fid)
    failing, targets, slow, last, n_started, n_blocks, failures = blocks(logged)
    timeout = "(timeout)" in r["failed"]
    named = [name for name in r["failed"] if not name.startswith("(")]
    restarts_targets = [t for t in targets if "--test transfer_restarts" in t]
    other_targets = [t for t in targets if "--test transfer_restarts" not in t]
    restarts_tests = failing.get("transfer_restarts", [])
    other_tests = {b: t for b, t in failing.items() if b != "transfer_restarts"}
    if r["verdict"] != "caught" or r["expect"] != "caught":
        problems.append(f"{fid}: verdict {r['verdict']}, expected {r['expect']}")
    if not timeout and n_started != n_blocks:
        problems.append(f"{fid}: {n_started} binaries started, {n_blocks} result blocks")
    kind = ("timeout only" if timeout and not named else
            "timeout and tests" if timeout else
            "no test named" if not named else
            "transfer_restarts only" if restarts_tests and not other_tests else
            "named tests")
    before = earlier.get(fid)
    before_tests = [n for n in before[2]["failed"] if not n.startswith("(")] if before else []
    rows.append((fid, r, kind, failing, targets, slow, last, failures, restarts_tests, other_tests,
                 before, before_tests, other_targets, restarts_targets))

counts = {}
for row in rows:
    counts[row[2]] = counts.get(row[2], 0) + 1
verdicts = {}
for r in results.values():
    verdicts[r["verdict"]] = verdicts.get(r["verdict"], 0) + 1
print(f"verdicts {verdicts}; kinds {counts}")
print(f"seed tests only: {[fid for fid, r in results.items() if r['seeds_only']]}")

print("\n== every fault: kind, seconds, the binaries with failing tests (count), earlier evidence")
for fid, r, kind, failing, targets, slow, last, failures, rt, ot, before, bt, _, _ in rows:
    now = set(n for tests in failing.values() for n in tests)
    overlap = len(now & set(bt))
    print(f"{fid:6} {kind:22} {r['seconds']:5}s  "
          + ", ".join(f"{b} {len(t)}" for b, t in sorted(failing.items()))
          + (f" | before ({before[1]} {before[2]['verdict']}): {len(bt)} tests, {overlap} of them fail now"
             if before else " | no earlier evidence"))

print("\n== reached the timeout")
for fid, r, kind, failing, targets, slow, last, failures, *_ in rows:
    if kind.startswith("timeout"):
        print(f"{fid}: {kind}; running when ended: {last}; over 60 s: {slow[-8:]}; named: "
              f"{[n for n in r['failed'] if not n.startswith('(')][:8]}")

print("\n== no failing test named")
for fid, r, kind, failing, targets, *_ in rows:
    if kind == "no test named":
        print(f"{fid}: {r['failed']}; targets failed {targets}")

print("\n== transfer_restarts failed (only target first)")
for fid, r, kind, failing, targets, slow, last, failures, rt, ot, before, bt, other_targets, rtargets in \
        sorted(rows, key=lambda row: row[2] != "transfer_restarts only"):
    if not rt and not rtargets:
        continue
    print(f"-- {fid} [{kind}] {r['path']}: {r['what']}")
    print(f"   transfer_restarts failing: {rt}")
    print(f"   other binaries failing: {ot or 'none'}; other targets failed: {other_targets or 'none'}")
    print(f"   before ({before[1] if before else '-'}): {bt[:6]}")
    for (binary, test), lines in failures.items():
        if binary == "transfer_restarts":
            print(f"   {test}: " + " | ".join(lines))

if problems:
    print("\n== PROBLEMS")
    for problem in problems:
        print(problem)
else:
    print("\n== no problem in the checks above (the faults caught by transfer_restarts alone are still looked at by hand)")
