"""Writes c3r-unmodified.py from c3-unmodified.py: the same two unmodified
checks, after the restore of 2026-11-11, with one change of reference:
each copy's test binaries are compared with those the same copy built of
c90d5e5 yesterday (~/diag-c3), and every one must be identical."""
import os

here = os.path.dirname(os.path.abspath(__file__))
text = open(os.path.join(here, "c3-unmodified.py"), encoding="utf-8").read()
changes = [
    ('"""Gate C on c90d5e5, first step (owner, 2026-11-10, 98 faults): the two unmodified',
     '"""Gate C on c90d5e5, after Windows restarted (2026-11-11), before FT29 and HR77:\n'
     'the copies were restored (c3r-restore.sh). As on 2026-11-10 (owner, 98 faults), the two unmodified'),
    ('''        before = json.load(open(os.path.expanduser(
            "~/diag-c2/tree0-binaries.json" if name == "tree0"
            else "~/evidence-9851c24/tree1-binaries-9851c24.json")))''',
     '''        # The reference is the build of the same copy of c90d5e5 before
        # the restart: a restore that left anything behind shows here.
        before = json.load(open(os.path.expanduser(f"~/diag-c3/{name}-binaries.json")))'''),
    ('''        if any(not k.startswith("transfer_restarts-") for k in other) or gone:
            wrong.append("a test binary other than transfer_restarts differs or is gone")''',
     '''        if other or new or gone or len(same) != 51:
            wrong.append("a test binary differs from the build of c90d5e5 before the restart")'''),
]
for old, new in changes:
    assert text.count(old) == 1, old[:60]
    text = text.replace(old, new, 1)
open(os.path.join(here, "c3r-unmodified.py"), "w", encoding="utf-8", newline="\n").write(text)
print("c3r-unmodified.py written")
