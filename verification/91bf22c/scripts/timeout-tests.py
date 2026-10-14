"""Prints the recorded failing tests of the given fault ids in every
result file named on the command line."""
import json
import sys

ids = set(sys.argv[1].split(","))
for path in sys.argv[2:]:
    data = json.load(open(path))
    results = data.get("results", data) if isinstance(data, dict) else data
    for r in results:
        if r["id"] in ids:
            print(path, r["id"], r["seconds"], r["failed"])
