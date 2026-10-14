#!/usr/bin/env python3
"""Mirror the Windows checkout into a native WSL directory.

Copies files whose size or mtime differ and removes files that no longer
exist in the source. Build outputs, git metadata and mutation copies are
not mirrored.
"""
import os
import shutil
import sys

SRC = "<repo>"
DST = os.path.expanduser("~/mono")
SKIP_DIRS = {".git", "target", "work", "corpus", "artifacts", "coverage"}


def walk(root):
    out = {}
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        rel = os.path.relpath(dirpath, root)
        for name in filenames:
            path = os.path.join(dirpath, name)
            key = os.path.normpath(os.path.join(rel, name))
            out[key] = path
    return out


def main():
    os.makedirs(DST, exist_ok=True)
    src = walk(SRC)
    dst = walk(DST)
    copied = removed = 0
    for key, path in src.items():
        target = os.path.join(DST, key)
        st = os.stat(path)
        try:
            dt = os.stat(target)
            same = dt.st_size == st.st_size and int(dt.st_mtime) == int(st.st_mtime)
            if not same and dt.st_size == st.st_size:
                # The same bytes with another time, such as a file a fault
                # trial restored: left as it is, so that its newer time
                # makes cargo build it again.
                with open(path, "rb") as a, open(target, "rb") as b:
                    same = a.read() == b.read()
        except FileNotFoundError:
            same = False
        if not same:
            os.makedirs(os.path.dirname(target), exist_ok=True)
            shutil.copy2(path, target)
            # A copy is newer than any build: cargo builds it again even
            # when the source is older than the last build.
            os.utime(target, None)
            copied += 1
    for key, path in dst.items():
        if key not in src:
            os.remove(path)
            removed += 1
    print(f"sync: {copied} copied, {removed} removed", file=sys.stderr)


if __name__ == "__main__":
    main()
