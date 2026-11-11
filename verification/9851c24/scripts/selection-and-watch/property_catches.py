"""Lists the faults a results file shows caught only by proptest properties.

A property's name is a fn declared inside a `proptest! { ... }` block of a
crate's sources or tests. Usage: property_catches.py <repo> <results.json>...
"""
import json
import os
import re
import sys

repo = sys.argv[1]


def properties(root):
    names = {}
    for base, dirs, files in os.walk(os.path.join(root, "crates")):
        dirs[:] = [d for d in dirs if d != "target"]
        for name in files:
            if not name.endswith(".rs"):
                continue
            path = os.path.join(base, name)
            text = open(path, encoding="utf-8").read()
            for match in re.finditer(r"proptest!\s*\{", text):
                depth, i = 1, match.end()
                while depth and i < len(text):
                    if text[i] == "{":
                        depth += 1
                    elif text[i] == "}":
                        depth -= 1
                    i += 1
                block = text[match.end():i]
                for fn in re.finditer(r"\bfn\s+([a-z_0-9]+)\s*\(", block):
                    names[fn.group(1)] = os.path.relpath(path, root)
    return names


props = properties(repo)
print(f"{len(props)} properties")
for results in sys.argv[2:]:
    data = json.load(open(results))
    only = []
    for item in data["results"]:
        if item["verdict"] != "caught":
            continue
        failed = [n.split("::")[-1] for n in item["failed"] if not n.startswith("(")]
        if failed and all(n in props for n in failed):
            only.append((item["id"], failed))
        elif not failed:
            only.append((item["id"], item["failed"]))
    print(f"{results}: {len(data['results'])} results, {len(only)} caught only by properties or unnamed")
    for fid, failed in only:
        print(f"  {fid}: {', '.join(failed)}")
