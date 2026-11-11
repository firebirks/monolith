"""Read-only reconciliation of the ID sets of audit_selection.py (owner,
2026-11-09): the universe split into four disjoint cells (selected and
run, selected and not run, run and not selected, neither), where the nine
expected survivors and the four faults with no certain attribution fall,
the 96 to run with no overlap, their patterns checked at the candidate as
the runner's precheck does, and what stays not run after them.
Writes the 96 ids, in faults.py order, to run96.txt.
Usage: reconcile_ids.py <same arguments as audit_selection.py>"""
import contextlib
import io
import os
import runpy
import sys

here = os.path.dirname(os.path.abspath(__file__))
sys.argv = ["audit_selection.py"] + sys.argv[1:]
with contextlib.redirect_stdout(io.StringIO()):
    g = runpy.run_path(os.path.join(here, "audit_selection.py"), run_name="__main__")

universe = g["universe"]
U = [f["id"] for f in universe]
assert len(U) == len(set(U)), "duplicate ids in the universe"
U = set(U)
S, ran, B, done = set(g["S"]), set(g["ran"]), set(g["B"]), set(g["done"])
evidence, expected, by_id = g["evidence"], g["expected"], g["by_id"]
assert S <= U and ran <= U, (S - U, ran - U)

cells = {
    "selected, run on the candidate": S & ran,
    "selected, not run": S - ran,
    "not selected, run on the candidate": ran - S,
    "not selected, not run": U - S - ran,
}
names = list(cells)
for i, a in enumerate(names):
    for b in names[i + 1:]:
        assert not cells[a] & cells[b], (a, b)
assert set().union(*cells.values()) == U
print(f"universe {len(U)} = " + " + ".join(f"{len(c)} {n}" for n, c in cells.items()))
print(f"run on the candidate {len(ran)} = gate B {len(B)} + Phase 7 part 1 {len(done)} "
      f"(overlap {len(B & done)})")
print(f"selected {len(S)}; not selected {len(U - S)} = "
      f"{len(cells['not selected, not run'])} not run + {len(cells['not selected, run on the candidate'])} run: "
      f"{' '.join(sorted(cells['not selected, run on the candidate']))}")
print(f"not run {len(U - ran)} = {len(cells['selected, not run'])} selected + "
      f"{len(cells['not selected, not run'])} not selected")

survivors = {i for i in U if expected(by_id[i]) == "survived"}
print("expected survivors:", len(survivors),
      {n: sorted(survivors & c) for n, c in cells.items() if survivors & c})


def attributable(i):
    """Caught, by named tests only, with no timeout or exit mark."""
    ev = evidence[i]
    named = [n for n in ev["failed"] if not n.startswith("(")]
    marks = [n for n in ev["failed"] if n.startswith("(")]
    return ev["verdict"] == "caught" and named and not marks


uncertain = {i for i in U - survivors if not attributable(i)}
print("expected caught, last evidence not attributable to named tests alone:", len(uncertain))
for i in sorted(uncertain):
    cell = next(n for n, c in cells.items() if i in c)
    print(f"  {i:5} {cell:36} {evidence[i]['seconds']:5}s {evidence[i]['failed']}")
four = uncertain - ran
assert four == {"MR25", "K4", "CS30", "Q1"}, four
assert not four & S, four & S
new = cells["selected, not run"]
run96 = new | four
assert len(run96) == len(new) + len(four) == 96
assert not run96 & ran and not run96 & survivors
print(f"to run: {len(new)} selected + {len(four)} not attributable = {len(run96)} unique, "
      f"none already run, none an expected survivor")

# The runner's precheck, at the candidate: each pair found once, applied in turn.
problems = []
for i in sorted(run96):
    f = by_id[i]
    text = g["show"](g["new"], f["path"])
    for old, new_text in f["pairs"]:
        if text.count(old) != 1:
            problems.append(f"{i}: pattern found {text.count(old)} times in {f['path']}")
        if old == new_text:
            problems.append(f"{i}: replacement changes nothing")
        text = text.replace(old, new_text, 1)
print("precheck at the candidate:", problems or "every pattern found once")

crates = sorted({c for i in run96 for c in by_id[i]["crates"]})
print("crates tested:", " ".join(crates))
order = [f["id"] for f in universe if f["id"] in run96]
seconds = sum(evidence[i]["seconds"] for i in order)
print(f"evidence seconds {seconds} (about {seconds * 1.68 / 2 / 3600:.1f} h with two workers at the "
      f"gate B rate, the timeouts bounded by the runner's own)")
after = U - ran - run96
print(f"after the run: run {len(ran | run96)}, not run {len(after)} "
      f"({len(after & survivors)} expected survivors, {len(after - survivors)} with a named killing test "
      f"in an unchanged file and the function not edited)")
assert all(attributable(i) for i in after - survivors)
with open(os.path.join(here, "run96.txt"), "w", encoding="ascii", newline="\n") as out:
    out.write(" ".join(order) + "\n")
print("ids in faults.py order written to run96.txt")
