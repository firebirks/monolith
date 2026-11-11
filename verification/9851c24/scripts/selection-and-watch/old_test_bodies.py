"""For each test of the older commit still present in the newer, compares
the body (whitespace ignored) and prints those that changed, with the
count of assertion lines before and after.
Usage: old_test_bodies.py <repo> <old commit> <new commit> [--show NAME]"""
import io
import re
import subprocess
import sys
import tarfile

repo, old, new = sys.argv[1:4]
show = sys.argv[5] if len(sys.argv) > 5 and sys.argv[4] == "--show" else None
HEAD = re.compile(r"#\[(?:tokio::)?test(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+([a-z_0-9]+)[^{]*\{")
PROP = re.compile(r"proptest!\s*\{")
FN = re.compile(r"\bfn\s+([a-z_0-9]+)\s*\([^{]*?\)\s*\{", re.S)


def body(text, start):
    depth, i = 1, start
    while depth and i < len(text):
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        i += 1
    return text[start:i - 1]


def tests(commit):
    data = subprocess.run(["git", "-C", repo, "archive", "--format=tar", commit, "crates", "tests"],
                          check=True, capture_output=True).stdout
    found = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as files:
        for member in files.getmembers():
            if not member.isfile() or not member.name.endswith(".rs"):
                continue
            text = files.extractfile(member).read().decode("utf-8")
            for match in HEAD.finditer(text):
                found[(match.group(1), member.name)] = body(text, match.end())
            for match in PROP.finditer(text):
                block = body(text, match.end())
                for fn in FN.finditer(block):
                    found[(fn.group(1), member.name)] = body(block, fn.end())
    return found


def norm(s):
    return re.sub(r"\s+", "", s)


def asserts(s):
    return len(re.findall(r"\b(?:prop_)?assert(?:_eq|_ne|_matches)?!|\.expect_err\(|\bpanic!|should_panic|\.is_err\(\)", s))


a, b = tests(old), tests(new)
byname_b = {}
for (name, path), text in b.items():
    byname_b.setdefault(name, []).append((path, text))
changed = []
for (name, path), text in sorted(a.items()):
    candidates = byname_b.get(name, [])
    if not candidates:
        continue
    same = [p for p, t in candidates if norm(t) == norm(text)]
    if same:
        continue
    newpath, newtext = candidates[0]
    changed.append((name, path, newpath, asserts(text), asserts(newtext), text, newtext))
print(f"old tests still present with a changed body: {len(changed)}")
for name, path, newpath, na, nb, text, newtext in changed:
    flag = "  FEWER CHECKS" if nb < na else ""
    print(f"  {name} ({path}{' -> ' + newpath if newpath != path else ''}): checks {na} -> {nb}{flag}")
    if show == name or show == "all":
        import difflib
        for line in difflib.unified_diff(text.splitlines(), newtext.splitlines(), lineterm="", n=1):
            print("     " + line)
