"""Prints, one line per phase, the faults of the given logs whose run
overlapped a time the mutation run was stopped (~/reverify-bbf-pauses.txt).
A stopped run keeps its clocks going, so the timeouts of such a fault can
fire for nothing; it has to run again. Each log line starts with the time
it was written, in seconds; a fault line ends with its duration."""
import os
import re
import sys

home = os.path.expanduser("~")
stops = []
start = None
pauses = os.path.join(home, "reverify-bbf-pauses.txt")
if os.path.exists(pauses):
    with open(pauses) as handle:
        for line in handle:
            parts = line.split()
            if len(parts) < 2:
                continue
            if parts[0] == "STOP" and start is None:
                start = int(parts[1])
            elif parts[0] == "CONT" and start is not None:
                stops.append((start, int(parts[1])))
                start = None
if start is not None:
    stops.append((start, float("inf")))

fault = re.compile(r"^(\d+) ([A-Z]+\d+)\s+(\w+)\s+(\d+)s  ")
by_phase = {}
for log in sys.argv[1:]:
    label = os.path.basename(log)[len("reverify-bbf-mutation-"):-len(".log")]
    phase = label.split("-")[0]
    with open(log) as handle:
        for line in handle:
            m = fault.match(line)
            if not m:
                continue
            end = int(m.group(1))
            begin = end - int(m.group(4)) - 1
            if any(begin < b and end > a for a, b in stops):
                by_phase.setdefault(phase, []).append(m.group(2))
for phase, ids in by_phase.items():
    print(phase, " ".join(ids))
