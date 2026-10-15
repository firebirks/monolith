# Packaging the desktop client

The desktop client is one binary, `monolith-desktop`, with its fonts and
icons compiled in. It needs no installer, no root and no service of its
own; it uses the Tor the system runs and never bundles one
(`docs/TOR_INTEGRATION.md`, `docs/DESIGN_QUESTIONS.md` P6-13).

## Binaries

- Development: `cargo run -p monolith-desktop`. With
  `--features screenshot` and `MONOLITH_SCREENSHOT=<file.png>` a build
  writes a picture of its window two seconds after it opens and closes;
  that feature is never part of a release.
- Release: `cargo build --release --locked -p monolith-desktop`, in
  `target/release/`. The archives below strip its symbols and change
  nothing else.

Both take `--data-dir <dir>` and `--config <file>` (absolute paths) in
place of the default places, for tests and for a second profile.

## Archives

| Platform | Command | Result |
| --- | --- | --- |
| Linux | `packaging/linux/build.sh` | `target/package/monolith-desktop-<version>-linux-<arch>.tar.gz` |
| Windows | `pwsh packaging/windows/build.ps1` | `target\package\monolith-desktop-<version>-windows-x86_64.zip` |

Each comes with a `.sha256` file. The Linux archive holds the binary, a
desktop entry, a 256 pixel icon drawn by the same code as the window's
(`cargo run -p monolith-desktop --example icon -- <file> [side]`), an
installer for the user's home (`install.sh`: `~/.local/bin`,
`~/.local/share/applications`, `~/.local/share/icons`), a README for
the user, `LICENSE`, the licences of the fonts and icons, and
`THIRD-PARTY-NOTICES.txt`. Its files are sorted, owned by nobody and dated
at the commit, so that the same commit and toolchain give the same
archive. The Windows archive holds the binary, a README, `LICENSE`, the
same licences and its own `THIRD-PARTY-NOTICES.txt`; the binary carries
no icon resource of its own (the window sets its icon when it opens).
Both scripts take the binary from the target directory cargo uses,
`CARGO_TARGET_DIR` included, and need Python 3 (`python3` on Linux,
`python` on Windows).

## Third-party notices

The binary links about 170 (Windows) to 250 (Linux) third-party crates,
under MIT, Apache-2.0, BSD, ISC, Zlib, BSL-1.0, Unicode-3.0 and the other
licences `deny.toml` allows; most of these ask for their copyright notice
and licence text in every copy, binaries included.
`python3 packaging/notices.py <target triple> <file>` writes them, and
both scripts run it for the host they build on:

1. the crates built into `monolith-desktop` for that target, with their
   versions and licence expressions: its normal dependencies and theirs,
   procedural macros included, without development and build
   dependencies, as `cargo tree -p monolith-desktop --edges normal`
   resolves them (the resolve of `cargo metadata` unites the features of
   every target and workspace member, and would name crates the binary
   does not contain);
2. the crates whose packages carry no licence file, with their authors;
3. every licence text their packages carry (`LICENSE*`, `LICENCE*`,
   `COPYING*`), each distinct text once with the crates that carry it;
4. their notice files (`NOTICE*`, `COPYRIGHT*`), likewise;
5. the standard texts, as SPDX publishes them (`packaging/licenses/`,
   license-list-data 3.27.0), of the licences named by the crates of
   item 2 and of the Rust standard library (MIT, Apache-2.0);
6. the licences of the fonts and icons
   (`crates/monolith-ui-kit/assets/licenses/`).

The generator stops with an error when a crate has no licence file and
names a licence whose standard text is not in `packaging/licenses/`; the
text is then added there from SPDX. Its output depends only on the lock
file, the target and the files it reads.

## What the binary needs at run time

- Linux: glibc of the build machine or newer, X11 or Wayland, OpenGL 2.1
  or OpenGL ES 2.0 (Mesa), libxkbcommon. The windowing libraries are
  loaded when the window opens, not linked. Optional: DejaVu Sans and
  Noto Sans CJK, from the fixed paths of `docs/adr/0012-design-system.md`.
- Windows: Windows 10 or 11, 64-bit, with an OpenGL 2.1 driver. Segoe UI,
  Microsoft YaHei and Malgun Gothic are used for scripts the bundled fonts
  lack, when present.
- Tor, as the system runs it: the Debian `tor` service, Tor Browser, or on
  Windows the Tor Expert Bundle. The user READMEs in `linux/README.txt` and
  `windows/README.txt` say how to point Monolith at each.

## Tails and Whonix

The archive runs from the user's home on both. On Tails the program has to
be kept in the persistent storage, and the control port of Tor is reached
through onion-grater with the profile in `integrations/tails/`; on Whonix
the profile and the firewall rule are in `integrations/whonix/`
(`docs/PLATFORM_TAILS.md`, `docs/PLATFORM_WHONIX.md`). Nothing in the
package changes how Monolith reaches Tor.
