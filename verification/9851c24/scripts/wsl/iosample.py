"""Samples the WSL machine once a second until it is ended, one line per
sample: the disk that holds the home directory (requests and MB per
second, mean wait of a write and of a flush, busy time, queue, requests
in flight), pressure stall of I/O, CPU and memory, dirty and writeback
memory, available memory, load, CPU time by kind, and what runs:
compilers, linkers, processes waiting in the kernel (state D), and the
test binaries of the copies with how many of their threads wait in the
kernel. Usage: iosample.py <out file>"""
import os
import sys
import time

out = open(sys.argv[1], "a", encoding="utf-8", buffering=1)
dev = os.stat(os.path.expanduser("~")).st_dev
MAJOR, MINOR = os.major(dev), os.minor(dev)
WORK = os.path.expanduser("~/p8/mutation/work/")
LINKERS = {"cc", "ld", "ld.lld", "collect2", "rust-lld", "ld.bfd", "ld.gold", "mold"}


def disk():
    for line in open("/proc/diskstats"):
        fields = line.split()
        if int(fields[0]) == MAJOR and int(fields[1]) == MINOR:
            return fields[2], [int(x) for x in fields[3:]]
    return "?", None


def pressure(kind):
    try:
        text = open(f"/proc/pressure/{kind}").read()
    except OSError:
        return None
    return {line.split()[0]: int(line.split()[-1].split("=")[1]) for line in text.splitlines()}


def meminfo():
    fields = {}
    for line in open("/proc/meminfo"):
        name, value = line.split(":", 1)
        fields[name] = int(value.split()[0])
    return fields


def cpu():
    return [int(x) for x in open("/proc/stat").readline().split()[1:9]]


def state_of(path):
    text = open(path).read()
    end = text.rindex(")")
    return text[text.index("(") + 1:end], text[end + 2]


def processes():
    rustc = link = waiting = 0
    tests = []
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            comm, state = state_of(f"/proc/{pid}/stat")
        except (OSError, ValueError):
            continue
        if state == "D":
            waiting += 1
        if comm == "rustc":
            rustc += 1
            continue
        if comm in LINKERS:
            link += 1
            continue
        try:
            exe = os.readlink(f"/proc/{pid}/exe")
        except OSError:
            continue
        if exe.startswith(WORK) and "/target/debug/deps/" in exe:
            tree = exe[len(WORK):].split("/")[0]
            name = os.path.basename(exe).rsplit("-", 1)[0]
            held = 0
            try:
                for tid in os.listdir(f"/proc/{pid}/task"):
                    if state_of(f"/proc/{pid}/task/{tid}/stat")[1] == "D":
                        held += 1
            except (OSError, ValueError):
                pass
            tests.append(f"{tree}:{name}(D{held})")
    return rustc, link, waiting, tests


def rate(now, before, field):
    if now is None or before is None:
        return float("nan")
    return now.get(field, 0) - before.get(field, 0)


name, before = disk()
kinds = ("io", "cpu", "memory")
psi_before = {kind: pressure(kind) for kind in kinds}
cpu_before = cpu()
t_before = time.monotonic()
out.write(f"# {time.strftime('%Y-%m-%dT%H:%M:%S')} disk {name} ({MAJOR}:{MINOR}), "
          f"{len(before)} fields; pressure {'yes' if psi_before['io'] else 'no'}\n")
while True:
    time.sleep(1)
    try:
        t_now = time.monotonic()
        ms = (t_now - t_before) * 1000
        seconds = ms / 1000
        _, now = disk()
        d = [a - b for a, b in zip(now, before)]
        psi_now = {kind: pressure(kind) for kind in kinds}
        psi = {}
        for kind in kinds:
            for level in ("some", "full"):
                psi[f"{kind}.{level}"] = rate(psi_now[kind], psi_before[kind], level) / (ms * 10)
        cpu_now = cpu()
        c = [a - b for a, b in zip(cpu_now, cpu_before)]
        total = sum(c) or 1
        mem = meminfo()
        load = open("/proc/loadavg").read().split()
        rustc, link, waiting, tests = processes()
        flushes = d[15] if len(d) > 16 else 0
        flush_ms = d[16] if len(d) > 16 else 0
        out.write(
            f"{time.strftime('%H:%M:%S')} load {load[0]} run {load[3]}"
            f" | r {d[0] / seconds:.0f}/s {d[2] * 512 / 1e6 / seconds:.1f}MB/s"
            f" w {d[4] / seconds:.0f}/s {d[6] * 512 / 1e6 / seconds:.1f}MB/s"
            f" wawait {d[7] / d[4] if d[4] else 0:.1f}ms"
            f" fl {flushes / seconds:.1f}/s flawait {flush_ms / flushes if flushes else 0:.1f}ms"
            f" util {d[9] / ms * 100:.0f}% aqu {d[10] / ms:.1f} inflight {now[8]}"
            f" | psi io {psi['io.some']:.0f}/{psi['io.full']:.0f}% cpu {psi['cpu.some']:.0f}%"
            f" mem {psi['memory.some']:.0f}/{psi['memory.full']:.0f}%"
            f" | dirty {mem['Dirty'] / 1024:.0f}MB wb {mem['Writeback'] / 1024:.0f}MB"
            f" avail {mem['MemAvailable'] / 1024:.0f}MB"
            f" | cpu us {(c[0] + c[1]) * 100 / total:.0f}% sy {(c[2] + c[5] + c[6]) * 100 / total:.0f}%"
            f" wa {c[4] * 100 / total:.0f}% id {c[3] * 100 / total:.0f}% st {c[7] * 100 / total:.0f}%"
            f" | rustc {rustc} link {link} D {waiting} | {' '.join(tests) or '-'}\n"
        )
        before, psi_before, cpu_before, t_before = now, psi_now, cpu_now, t_now
    except Exception as error:  # noqa: BLE001 - a sample lost is noted, not fatal
        out.write(f"{time.strftime('%H:%M:%S')} sample failed: {error!r}\n")
