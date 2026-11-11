import json, re, os
home = os.path.expanduser("~")
R = "<work>/results"
ran = {}
for r in json.load(open(f"{R}/results-B-9851c24.json"))["results"]: ran[r["id"]] = "B " + r["verdict"]
for line in open(f"{home}/final8-mutation-phase7-part1.log"):
    m = re.match(r"^([A-Z]+\d+)\s+(\w+)", line)
    if m: ran[m.group(1)] = "part1 " + m.group(2)
for r in json.load(open(f"{home}/diag-c3/final/results-C3-merged.json"))["results"]: ran[r["id"]] = "C " + r["verdict"]
moved = "HR17 HR18 HR19 HR20 HR85 MR8 MR39 MR44 MR54 HR32 HR76 UI110 HR9 HR10 HR11 HR12 HR13 HR14 HR68 HR69 HR70 HR71".split()
print(len(moved))
for m in moved: print(m, ran.get(m, "NOT RUN"))
