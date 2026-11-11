"""Lists the tests (fns marked #[test] or #[tokio::test], and fns inside
proptest! blocks) of the crates at two commits, and prints the names the
older commit has and the newer lacks, with the file each was in.
Usage: old_tests_diff.py <repo> <old commit> <new commit>"""
import io
import re
import subprocess
import sys
import tarfile

repo, old, new = sys.argv[1:4]
ATTR = re.compile(r"#\[(?:tokio::)?test(?:\([^)]*\))?\]\s*(?:#\[[^\]]*\]\s*)*(?:async\s+)?fn\s+([a-z_0-9]+)")


def tests(commit):
    data = subprocess.run(["git", "-C", repo, "archive", "--format=tar", commit, "crates", "tests"],
                          check=True, capture_output=True).stdout
    found = {}
    with tarfile.open(fileobj=io.BytesIO(data)) as files:
        for member in files.getmembers():
            if not member.isfile() or not member.name.endswith(".rs"):
                continue
            text = files.extractfile(member).read().decode("utf-8")
            for match in ATTR.finditer(text):
                found.setdefault(match.group(1), []).append(member.name)
            for match in re.finditer(r"proptest!\s*\{", text):
                depth, i = 1, match.end()
                while depth and i < len(text):
                    depth += {"{": 1, "}": -1}.get(text[i], 0)
                    i += 1
                for fn in re.finditer(r"\bfn\s+([a-z_0-9]+)\s*\(", text[match.end():i]):
                    found.setdefault(fn.group(1), []).append(member.name)
    return found


a, b = tests(old), tests(new)
print(f"{old}: {sum(len(v) for v in a.values())} tests, {len(a)} names")
print(f"{new}: {sum(len(v) for v in b.values())} tests, {len(b)} names")
gone = sorted(set(a) - set(b))
print(f"names gone: {len(gone)}")
for name in gone:
    print(f"  {name}  ({', '.join(a[name])})")
moved = sorted(n for n in set(a) & set(b) if sorted(a[n]) != sorted(b[n]))
print(f"names whose files changed: {len(moved)}")
for name in moved:
    print(f"  {name}: {a[name]} -> {b[name]}")
