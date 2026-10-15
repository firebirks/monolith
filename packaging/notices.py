#!/usr/bin/env python3
"""Writes THIRD-PARTY-NOTICES.txt for a package of the desktop client.

Usage: python3 packaging/notices.py <target triple> <output file>

The crates are those cargo builds into monolith-desktop for the target:
its normal dependencies and theirs (procedural macros included), without
development and build dependencies. `cargo tree` names them, with the
features `cargo build -p monolith-desktop` turns on; `cargo metadata
--filter-platform <target>` gives the version, licence expression and
folder of each. The resolve of cargo metadata alone would name crates the
program does not contain: it unites the features of every target and of
every member of the workspace, and lists an optional dependency that a
feature names weakly (`name?/feature`) even when nothing turns it on
(wgpu and tiff, for example).

For each crate the file gives its version and licence expression, then
every licence text and notice file its package carries (LICENSE*,
LICENCE*, COPYING*, NOTICE*, COPYRIGHT*), each distinct text once with the
crates that carry it. A crate whose package carries no licence text is
covered by the standard texts of the licences it names, kept in
packaging/licenses/ as SPDX publishes them. The licences of the fonts and
icons compiled into the program come last.

The output depends only on the lock file, the target and the files read;
it fails rather than leave a crate without a licence text.
"""

import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
ROOT = "monolith-desktop"
STANDARD = os.path.join(HERE, "licenses")
ASSETS = os.path.join(REPO, "crates", "monolith-ui-kit", "assets", "licenses")
LICENCE_FILES = ("license", "licence", "copying")
NOTICE_FILES = ("notice", "copyright")


def metadata(target):
    out = subprocess.run(
        [
            "cargo", "metadata", "--format-version", "1", "--locked",
            "--filter-platform", target,
        ],
        cwd=REPO,
        check=True,
        capture_output=True,
    ).stdout
    return json.loads(out)


def built(target):
    """The packages built into monolith-desktop for `target`, as (name,
    version), with features resolved as `cargo build -p monolith-desktop`
    resolves them."""
    out = subprocess.run(
        [
            "cargo", "tree", "-p", ROOT, "--target", target, "--edges", "normal",
            "--locked", "--prefix", "none", "--format", "{p}",
        ],
        cwd=REPO,
        check=True,
        capture_output=True,
    ).stdout.decode("utf-8", errors="replace")
    found = set()
    for line in out.splitlines():
        words = line.split()
        if len(words) >= 2 and words[1].startswith("v"):
            found.add((words[0], words[1][1:]))
    return found


def crates(meta, target):
    """The third-party packages built into monolith-desktop, sorted."""
    members = set(meta["workspace_members"])
    own = {package["name"] for package in meta["packages"] if package["id"] in members}
    by_version = {}
    for package in meta["packages"]:
        if package["id"] not in members:
            by_version.setdefault((package["name"], package["version"]), []).append(package)
    found = []
    for name, version in built(target):
        if name in own:
            continue
        packages = by_version.get((name, version), [])
        if len(packages) != 1:
            sys.exit(f"{name} {version}: {len(packages)} packages in cargo metadata")
        found.extend(packages)
    return sorted(found, key=lambda package: (package["name"], version_key(package)))


def version_key(package):
    parts = re.split(r"[.+-]", package["version"])
    return [(0, int(part), "") if part.isdigit() else (1, 0, part) for part in parts]


def read_text(path):
    with open(path, "rb") as handle:
        data = handle.read()
    try:
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        text = data.decode("latin-1")
    # A byte order mark is not text.
    text = text.lstrip(chr(0xFEFF)).replace("\r\n", "\n").replace("\r", "\n")
    lines = [line.rstrip() for line in text.split("\n")]
    while lines and not lines[0]:
        lines.pop(0)
    while lines and not lines[-1]:
        lines.pop()
    return "\n".join(lines)


def files_of(package):
    """The licence and notice files of a package: (name, path, is notice)."""
    directory = os.path.dirname(package["manifest_path"])
    found = []
    for name in sorted(os.listdir(directory)):
        path = os.path.join(directory, name)
        lower = name.lower()
        if not lower.startswith(LICENCE_FILES + NOTICE_FILES):
            continue
        notice = lower.startswith(NOTICE_FILES)
        if os.path.isfile(path):
            found.append((name, path, notice))
        elif os.path.isdir(path):
            for inner in sorted(os.listdir(path)):
                if os.path.isfile(os.path.join(path, inner)):
                    found.append((f"{name}/{inner}", os.path.join(path, inner), notice))
    named = package.get("license_file")
    if named:
        path = os.path.normpath(os.path.join(directory, named))
        if all(os.path.normpath(item[1]) != path for item in found):
            found.append((named, path, False))
    return found


def licence_ids(expression):
    """The licences an SPDX expression names; `/` is the old form of OR."""
    words = re.split(r"[\s()/]+", expression)
    return sorted({word for word in words if word and word not in ("AND", "OR", "WITH")})


def section(out, title):
    out.append("")
    out.append(title)
    out.append("-" * len(title))
    out.append("")


def text_block(out, heading, carriers, text):
    out.append("=" * 72)
    out.append(heading)
    for carrier in carriers:
        out.append(f"  {carrier}")
    out.append("=" * 72)
    out.append("")
    out.append(text)
    out.append("")


def label(package):
    return f"{package['name']} {package['version']}"


def write(target, output):
    meta = metadata(target)
    found = crates(meta, target)
    version = next(
        package["version"] for package in meta["packages"]
        if package["id"] in meta["workspace_members"] and package["name"] == ROOT
    )

    # Texts that differ only in spacing and line breaks are one text, given
    # as the first crate in the list carries it.
    texts = {}
    notices = {}
    without = []
    for package in found:
        expression = package.get("license") or ""
        has_licence = False
        for name, path, notice in files_of(package):
            text = read_text(path)
            if not text:
                continue
            into = notices if notice else texts
            entry = into.setdefault(" ".join(text.split()), (text, []))
            entry[1].append(f"{label(package)}: {name}")
            has_licence |= not notice
        if not has_licence:
            if not expression:
                sys.exit(f"{label(package)} states no licence and carries no licence file")
            without.append(package)

    standard = {}
    for package in without:
        for licence in licence_ids(package["license"]):
            path = os.path.join(STANDARD, f"{licence}.txt")
            if not os.path.isfile(path):
                sys.exit(f"{label(package)}: no standard text of {licence} in {STANDARD}")
            standard.setdefault(licence, []).append(label(package))

    width = max(len(label(package)) for package in found) + 2
    out = [
        f"Third-party notices of Monolith {version} for {target}",
        "=" * len(f"Third-party notices of Monolith {version} for {target}"),
        "",
        "The program monolith-desktop contains the code of the Rust crates",
        "listed in section 1, compiled into it, and the fonts and icons of",
        "section 6. Each crate is used under the licence named beside it; where",
        "it offers a choice (OR), any of the licences named can be chosen. The",
        "licence texts and notices of the crates follow, each text once with the",
        "crates whose packages carry it. Crates whose packages carry no licence",
        "text are listed in section 2 and covered by the standard texts of",
        "section 5. The program also contains the Rust standard library, which",
        "the Rust project distributes under the MIT License or the Apache",
        "License 2.0, both in section 5.",
        "",
        "Made by packaging/notices.py from the packages of the crates as cargo",
        "resolves them for this target.",
    ]

    section(out, f"1. Crates ({len(found)})")
    for package in found:
        licence = package.get("license") or package.get("license_file")
        out.append(f"{label(package):<{width}}{licence}")

    section(out, f"2. Crates whose packages carry no licence text ({len(without)})")
    for package in without:
        authors = ", ".join(package.get("authors") or []) or "not named"
        out.append(f"{label(package):<{width}}{package['license']}")
        out.append(f"{'':<{width}}authors: {authors}")

    section(out, f"3. Licence texts of the crates ({len(texts)})")
    for number, (text, carriers) in enumerate(texts.values(), start=1):
        text_block(out, f"3.{number}, carried by:", carriers, text)

    section(out, f"4. Notices of the crates ({len(notices)})")
    for number, (text, carriers) in enumerate(notices.values(), start=1):
        text_block(out, f"4.{number}, carried by:", carriers, text)

    # The standard library's licences are always given.
    for licence in ("MIT", "Apache-2.0"):
        standard.setdefault(licence, [])
    section(out, f"5. Standard licence texts ({len(standard)})")
    for licence in sorted(standard):
        carriers = standard[licence]
        if licence in ("MIT", "Apache-2.0"):
            carriers = carriers + ["the Rust standard library"]
        text = read_text(os.path.join(STANDARD, f"{licence}.txt"))
        text_block(out, f"{licence}, for:", carriers, text)

    fonts = sorted(
        name for name in os.listdir(ASSETS) if os.path.isfile(os.path.join(ASSETS, name))
    )
    section(out, f"6. Fonts and icons ({len(fonts)})")
    out.append("See crates/monolith-ui-kit/assets/README.md of the source for")
    out.append("where each comes from. The same texts are in the folder licenses.")
    out.append("")
    for name in fonts:
        text_block(out, name, [], read_text(os.path.join(ASSETS, name)))

    with open(output, "w", encoding="utf-8", newline="\n") as handle:
        handle.write("\n".join(out).rstrip("\n") + "\n")
    print(f"{output}: {len(found)} crates, {len(without)} without a licence text")


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: python3 packaging/notices.py <target triple> <output file>")
    write(sys.argv[1], sys.argv[2])


if __name__ == "__main__":
    main()
