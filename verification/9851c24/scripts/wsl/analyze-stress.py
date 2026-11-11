"""Read only: the results of stress-restarts.py. For each round and
process: exit, duration, CPU, peak memory, the result line and the tests
reported over 60 s; over all of them, the ranges; from io.log, the CPU
pressure, load, idle and steal, and the memory available, while the
rounds ran. Usage: analyze-stress.py <out dir>"""
import collections
import glob
import json
import os
import re
import sys

out = sys.argv[1]
records = []
for path in sorted(glob.glob(os.path.join(out, "round*", "proc*.json"))):
    records.append(json.load(open(path)))
for r in records:
    print(f"round {r['round']:2} p{r['index']} exit {r['exit']} {'TIMEOUT ' if r['timed_out'] else ''}"
          f"{r['seconds']:6.0f}s user {r['user_s']:5.0f}s sys {r['system_s']:4.0f}s "
          f"rss {r['maxrss_mb']}MB | {(r['result'] or '-')[:52]} | over 60 s: {len(r['over_60s'])}")
if records:
    def span(key):
        values = [r[key] for r in records]
        return f"{min(values)}-{max(values)}"
    print(f"== {len(records)} processes in {len({r['round'] for r in records})} rounds; "
          f"exit codes {collections.Counter(r['exit'] for r in records)}; timeouts {sum(r['timed_out'] for r in records)}")
    print(f"   seconds {span('seconds')}, user CPU {span('user_s')}, rss {span('maxrss_mb')} MB")
    slow = collections.Counter(name for r in records for name in r["over_60s"])
    print("   tests over 60 s (processes that reported each):")
    for name, count in slow.most_common():
        print(f"     {count:3} {name}")
    failed = collections.Counter(name for r in records for name in r["failed"])
    print(f"   tests failed: {dict(failed) or 'none'}")

SAMPLE = re.compile(r"^\d\d:\d\d:\d\d load (?P<load>[\d.]+) .*?psi io \d+/\d+% cpu (?P<psi>\d+)% .*?"
                    r"avail (?P<avail>\d+)MB .*? id (?P<idle>\d+)% st (?P<steal>\d+)%")
rows = []
for line in open(os.path.join(out, "io.log"), encoding="utf-8"):
    match = SAMPLE.match(line)
    if match and "transfer_restarts" in line:
        rows.append({k: float(v) for k, v in match.groupdict().items()})
if rows:
    print(f"== io.log, {len(rows)} samples with a test process running")
    for key in ("load", "psi", "idle", "steal", "avail"):
        values = [row[key] for row in rows]
        print(f"   {key}: mean {sum(values) / len(values):.1f} min {min(values):.0f} max {max(values):.0f}")
