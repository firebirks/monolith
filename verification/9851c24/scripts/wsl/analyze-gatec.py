"""Read only: for every cycle of the replay of the start of gate C, when
the build of each unmodified run ended, when transfer_restarts ran in it
and how long, its tests that ran over 60 s, and the I/O and load sampled
during the build, during transfer_restarts and during the rest of the
run. Usage: analyze-gatec.py <diag dir>"""
import glob
import os
import re
import sys

diag = sys.argv[1]
SAMPLE = re.compile(
    r"^(?P<t>\d\d:\d\d:\d\d) load (?P<load>[\d.]+) .*? w \d+/s (?P<wmb>[\d.]+)MB/s"
    r" wawait (?P<wawait>[\d.]+)ms fl (?P<fl>[\d.]+)/s flawait (?P<flawait>[\d.]+)ms"
    r" util (?P<util>\d+)% aqu (?P<aqu>[\d.]+) .*? psi io (?P<iosome>\d+)/(?P<iofull>\d+)%"
    r" cpu (?P<cpusome>\d+)% .*? dirty (?P<dirty>\d+)MB wb (?P<wb>\d+)MB .*? wa (?P<wa>\d+)%"
    r" id (?P<idle>\d+)% .*? \| rustc (?P<rustc>\d+) link (?P<link>\d+) D (?P<d>\d+) \| (?P<tests>.*)$")


def seconds(clock):
    h, m, s = clock.split(":")
    return int(h) * 3600 + int(m) * 60 + float(s)


def samples(path):
    rows = []
    for line in open(path, encoding="utf-8"):
        match = SAMPLE.match(line)
        if match:
            row = match.groupdict()
            row["s"] = seconds(row["t"])
            held = re.search(r"transfer_restarts\(D(\d+)\)", row["tests"])
            row["trD"] = int(held.group(1)) if held else None
            rows.append(row)
    return rows


def window(rows, start, end):
    chosen = [r for r in rows if start <= r["s"] <= end]
    if not chosen:
        return "no samples"
    out = [f"{len(chosen)} samples"]
    for key, unit in (("wmb", "MB/s"), ("wawait", "ms"), ("fl", "/s"), ("flawait", "ms"),
                      ("util", "%"), ("aqu", ""), ("iosome", "%"), ("iofull", "%"), ("cpusome", "%"),
                      ("dirty", "MB"), ("wb", "MB"), ("wa", "%"), ("idle", "%"), ("load", "")):
        values = [float(r[key]) for r in chosen]
        out.append(f"{key} mean {sum(values) / len(values):.1f} max {max(values):.1f}{unit}")
    held = [r["trD"] for r in chosen if r["trD"] is not None]
    if held:
        out.append(f"transfer_restarts threads in D: in {sum(1 for h in held if h)} of {len(held)} samples, max {max(held)}")
    return "; ".join(out)


for cycle_dir in sorted(glob.glob(os.path.join(diag, "cycle*"))):
    print(f"==== {os.path.basename(cycle_dir)}")
    io = samples(os.path.join(cycle_dir, "io.log"))
    for stamped in sorted(glob.glob(os.path.join(cycle_dir, "*-unmodified.stamped"))):
        lines = open(stamped, encoding="utf-8").read().splitlines()
        name = os.path.basename(stamped)
        times = [seconds(line.split()[0]) for line in lines]
        start = times[0] if times else 0
        finished = next((seconds(l.split()[0]) for l in lines if " E " in l and "Finished `test`" in l), None)
        compiling = next((seconds(l.split()[0]) for l in lines if " E " in l and "Compiling " in l), None)
        running = [(seconds(l.split()[0]), l) for l in lines if re.search(r" E +Running ", l)]
        restarts = next((i for i, (_, l) in enumerate(running) if "transfer_restarts" in l), None)
        print(f"-- {name}: first line {lines[0][:12] if lines else '?'}, last {lines[-1][:12] if lines else '?'}")
        if compiling is not None and finished is not None:
            print(f"   build {finished - compiling:.0f}s; I/O during the build: {window(io, compiling, finished)}")
        if restarts is not None:
            r_start = running[restarts][0]
            r_end = running[restarts + 1][0] if restarts + 1 < len(running) else times[-1]
            gap = r_start - finished if finished is not None else None
            result = next((l for l in lines if l.split()[0] and " O test result" in l
                           and r_start <= seconds(l.split()[0]) <= r_end + 1), "?")
            print(f"   transfer_restarts from {running[restarts][1][:12]} ({gap:.0f}s after the build ended) "
                  f"for {r_end - r_start:.0f}s: {result[13:].strip()}")
            slow = [l[13:].strip() for l in lines
                    if "has been running for over 60 seconds" in l and r_start <= seconds(l.split()[0]) <= r_end]
            print(f"   over 60 s: {len(slow)} {[s.split()[2] for s in slow]}")
            print(f"   I/O during transfer_restarts: {window(io, r_start, r_end)}")
            if finished is not None:
                print(f"   I/O between the build and transfer_restarts: {window(io, finished, r_start)}")
