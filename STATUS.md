# Status of the Phase 9 work

Written on 2026-11-11, when Phase 9 started, so that the work can be
picked up on another machine. This file describes work in progress on the
branch `phase-9-beta-readiness`. It is removed when Phase 9 is closed; it
does not belong on `main`.

Branch: `phase-9-beta-readiness`, created from `main` at `9882291`
(`phase-8-complete`). The production code of Phase 8 was verified on
`9851c24`, its tests and tooling on `c90d5e5`; `c0e8a6d` closed its
documents. The outputs are on `verification-9851c24`.

This file is the result of P9-A, the readiness audit, after an
independent review of its first draft (section 12), and the record of
the work approved since: P9-B0 (section 13), the maintenance of `main`
(section 14), and the update of the fuzz lockfile with the new minimum of
`rustix` (section 15). Of the workstreams after P9-A only P9-B0 was
approved; it is done and published. P9-S and the later workstreams have
not started. Section 9 lists the decisions asked of the owner and those
taken.

## 1. What Phase 9 is

Beta readiness: make the verified system something that people
unfamiliar with the project can install, set up, understand, use,
update and troubleshoot in a controlled external beta, without weakening
a security guarantee. No new protocol feature; no change to the
cryptographic protocols, the Tor session semantics, the storage formats
or the transfer rules to suit the interface. A change to a
security-critical invariant needs a design review of its own first.

Rules that hold until the phase is closed:

- Work on this branch only, in new local commits. Nothing is pushed,
  merged, tagged or released unless the owner asks for it. No history is
  rewritten. `phase-8-file-transfer`, `verification-9851c24`,
  `phase-8-complete` and every earlier tag and verification branch stay
  as they are.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 8 are a frozen baseline: no old test or fault is weakened;
  the nine expected survivors and the faults caught by the timeout alone
  keep their classification unless new evidence asks otherwise.
- Priorities, in this order: security guarantees; no irreversible loss of
  an identity or of messages; anonymity and no accidental network
  exposure; failures a user can understand and recover from; installation
  and maintenance; usability and accessibility; performance where
  measured.
- Mutation testing is risk-based and bounded (section 8): no campaign runs
  without the owner's approval, and the final gate proposes one with an
  exact manifest and cost.
- Each approved workstream ends with its results presented to the owner
  before the next one starts.
- No private key, identity data, session secret or runtime state goes into
  a commit or into published evidence. No recruitment of testers and no
  distribution of binaries without the owner.

## 2. Baseline

| Reference | Commit |
| --- | --- |
| Verified production | `9851c24` |
| Verified tests and tooling | `c90d5e5` |
| Phase 8 documentation closure | `c0e8a6d` |
| Published `main`, `phase-8-complete` | `9882291` |
| Verification outputs | `verification-9851c24` (`33d88aa`) |

`main` was clean and equal to `origin/main` when this branch was made.
Since then `main` has two commits that this branch does not have, both
approved by the owner apart from Phase 9 (section 14): `88359d4`, the
lockfile of P9-B0, and `f813528`, a new README. `phase-8-complete` and the
Phase 8 branches are unchanged.

CI on `main` and on the Phase 8 branches failed in one job after the
publication: `cargo deny check` reported RUSTSEC-2026-0486, published after
the final verification, an unsound handling of zero-length buffers in the
extended-attribute functions of `rustix::fs` (`getxattr`, `listxattr` and
their variants) in `rustix` 1.1.5, fixed in 1.1.6 (MSRV 1.65). `rustix` is
a direct dependency of `monolith-storage` on Unix (the file operations of
the profile, its lock, the user id) and a development dependency of
`monolith-desktop` on Unix (its tests); it also comes through the window
stack. Monolith calls none of the affected functions, and none of the
crates that depend on `rustix` 1.1.5 calls them in its sources.
`cargo audit` passed because it reports unsound advisories as warnings.
fmt, clippy, doc, the tests, MSRV, stable and the fuzz build passed. The
earlier failures of CI after Phases 6 and 7 were other advisories (tokio,
ctutils, clipboard-win), handled in Phase 8 (`3b6d63d`). P9-B0 updated
`rustix` to 1.1.6 on this branch, and the same lockfile went to `main`
(sections 13 and 14); CI is green on both. The earlier runs of the Phase 8
branches and of `phase-8-complete` stay as they were.

Approved deferrals that bear on a beta: BL14 to BL17 (tests with screen
readers, shaping and bidi, a message list reachable by keyboard, unread
counts kept), BL18 (blocking a key without a record), BL20 (finding the
Tor cookie by the name Tor gives), BL21 (installers and signing), BL22
(the client tried by hand on Tails and Whonix), BL53, BL54
(`DESIGN_QUESTIONS.md` 14.2), and in Phase 8 a file chooser of the
system, previews, folders, several files in one offer and files above
256 MiB (15.2, P8-7). The known limits of Phase 8 stay as recorded: the
lock of the disk has no order (15.4), the former hostile-sender test
anomaly, the timeout-only catches and the nine expected survivors (15.5).
Open protocol question that bears on backups: P3, the epoch of an
identity restored from an old backup (`PROTOCOL.md`, "Needs review").

## 3. Readiness inventory

Classes: VERIFIED (a verification record shows it, in the environment
named); UNVERIFIED (implemented, not verified in a supported
environment); PARTIAL; MISSING; DEFERRED (an approved decision). The
records are `docs/DESIGN_QUESTIONS.md` 13.3, 14.3 and 15.5 and the
branches `verification-*`. The environments of Phase 8 were a Windows 11
host, a Debian 12 container and Debian 13 in WSL; nothing ran on a real
Tails or Whonix, on Windows 10, or on a clean machine.

### 3.1 Distribution and release

| Item | Class | Evidence and remark |
| --- | --- | --- |
| Linux archive (binary, desktop entry, icon, `install.sh`, notices), files sorted, owned by nobody, dated at the commit | VERIFIED as built | `packaging/linux/build.sh`; built in WSL Debian 13 only, so it needs that glibc or newer; it is not a beta artifact |
| Windows zip (binary, README, notices) | VERIFIED as built; the packaged client ran idle on the host | `packaging/windows/build.ps1`; `Compress-Archive` keeps file times: not reproducible |
| The packaged binaries on a clean machine | MISSING | no record of an install and first run outside the development machines |
| `install.sh` for the user's home | UNVERIFIED | no uninstall script; nothing says to close Monolith before an update |
| Third-party notices | VERIFIED | `packaging/notices.py`, built into both archives |
| SHA-256 files | VERIFIED as written | shipped next to the archive: they detect corruption, not substitution |
| Signed manifest, signed tags, key, SBOM, `cargo auditable`, path remapping | MISSING | planned in `ARCHITECTURE.md` 11 (minisign proposed); no `--remap-path-prefix`, so two builds agree only at the same path |
| Installer (MSI, MSIX, Inno, AppImage, deb, Flatpak), Authenticode | DEFERRED | BL21 |
| Release version | MISSING | the workspace version is `0.0.0`; the client does not show it (`monolith-ui/src/settings.rs`) |
| Release pipeline, Windows CI | MISSING | CI runs on `ubuntu-latest` only; no Windows job, no packaging job; Windows results come from recorded manual runs (15.5) |
| Update check or updater | MISSING by design | none in the code; `ARCHITECTURE.md` 11 and the READMEs say no update checks |
| Security contact | MISSING | `SECURITY.md` still says "design phase", no release, contact and key to come; it sends reporters to the private vulnerability reporting of the repository host, which is off for this repository (checked on 2026-11-11), so no private channel works today (section 14.3, D17) |

### 3.2 Tor

| Item | Class | Evidence and remark |
| --- | --- | --- |
| Tor never started, bundled or managed by Monolith | VERIFIED by construction | `TOR_INTEGRATION.md`, P6-13; the user runs Tor Browser, the Expert Bundle or the system `tor` |
| Endpoints only loopback or an absolute Unix path; SAFECOOKIE only, cookie checked | VERIFIED | `monolith-tor` config and auth tests; the cookie's permissions are checked on Unix only |
| SOCKS proof before a dial (the port is Tor's) | VERIFIED on Linux | private network step 21; off on Tails and Whonix |
| Presets for Tor Browser (Desktop, OneDrive Desktop) and the Expert Bundle on Windows, system Tor and Tor Browser on Linux; a change applies after a restart | UNVERIFIED on Windows | `monolith-desktop/src/places.rs`; the private network runs on Linux only; no real Tor Browser or Expert Bundle run is recorded |
| Tor stopped, restarted; messages wait | VERIFIED on Linux | private network restart cycles and step 26 |
| No clearnet, DNS or UDP (T-NET-1) | VERIFIED for the CLI and the Tor test paths on Linux; MISSING for the desktop binary and for Windows | `tests/network/fail-closed.sh` traces `monolith` and the `monolith-tor` tests; the private network traces node runs and the headless example `tor_smoke`, not the packaged window; static bans in `clippy.toml` and `deny.toml`; links are never opened |
| Finding the cookie by the name Tor gives | DEFERRED | BL20 |

### 3.3 Identity and data

| Item | Class | Evidence and remark |
| --- | --- | --- |
| Identity creation under a passphrase; several identities; deletion | VERIFIED | runtime and core tests, private network; the deletion dialog says it cannot be undone |
| Passphrase rules | PARTIAL | not empty, bounded; "a" is accepted; the KDF defaults were measured on the development machine only (`STORAGE.md`) |
| Forgotten passphrase | MISSING by design | nothing recovers it; the welcome screen says "Nothing can be recovered without it." |
| Passphrase change | MISSING in the product | `Vault::change_passphrase` exists and is tested; nothing calls it |
| Backup and restore of an identity | MISSING | `STORAGE.md` 7 calls a copy of the vault made while Monolith is stopped a backup, and lists three hazards: a restored epoch lower than one issued later (P3, open), two running copies, an old copy that still opens with the old passphrase; the generation does not detect a rollback; `Installation::restore_identity` is called by its tests only; nothing in the client or the READMEs tells a user how to keep a copy |
| Transport-key rotation | PARTIAL | core and private network steps 6 and 32; started only by the development CLI; `THREAT_MODEL.md` relies on it as the answer to a stolen key |
| Profile and settings paths | VERIFIED by unit tests | Windows `%LOCALAPPDATA%\Monolith\Profile` and `desktop.conf`; Linux `$XDG_DATA_HOME/monolith` and `$XDG_CONFIG_HOME/monolith/desktop.conf`; Tails Persistent Storage; only local absolute paths; Settings > Data shows the profile folder |
| Unix permissions and ownership; single-instance lock | VERIFIED | 0600/0700 at creation, a wider profile refused |
| Windows ACLs | MISSING, stated | the profile relies on the ACL of the user's AppData; a `--data-dir` elsewhere inherits that place's ACL |
| A profile of a newer format | PARTIAL | refused without a change, but shown as "The profile is damaged" with advice to move the folder elsewhere and start again (`OpenError::Invalid` to `Failure::Damaged`) |
| What is not encrypted | VERIFIED, stated | `desktop.conf`, saved cards (they hold an invitation capability, saved to Downloads, which OneDrive often redirects), received files once saved, a partial plaintext file after a crash during a save |
| Uninstall and data | PARTIAL | data are outside the program folder and survive; the Windows README says to delete the data folder with no word that this is the only copy of the identities; nothing in Settings > Data says so either |

### 3.4 Operational privacy

| Item | Class | Evidence and remark |
| --- | --- | --- |
| Logging | VERIFIED: none | no logging crate; the client says "No logs"; T-LOG-1 has nothing to run against |
| Errors shown as fixed sentences, never Debug text | VERIFIED | `monolith-ui` `Failure`, `DESKTOP.md` |
| Debug output of secrets redacted | VERIFIED by unit tests | S18; not shown for panic messages |
| Panic output | UNVERIFIED | the default hook; stderr hidden on Windows by the window subsystem |
| Core dumps, Windows error reports, swap, hibernation | MISSING, stated | no `setrlimit` or `PR_SET_DUMPABLE`; `CRYPTOGRAPHY.md` and `STORAGE.md` say keys and plaintext can reach them |
| Secrets in memory | PARTIAL, stated | passphrases and keys zeroized; message text not; no memory locking |
| Clipboard, notifications, window title, recent files, external links | VERIFIED | clipboard only on an explicit action; no OS notification; the title carries a count only; no recent files; links never opened |
| Command lines | VERIFIED | the client takes only `--data-dir` and `--config`; the CLI takes a passphrase file, not a passphrase |

### 3.5 Desktop experience and accessibility

| Item | Class | Evidence and remark |
| --- | --- | --- |
| Welcome, create profile, use without saving, where the profile is kept | VERIFIED | fixtures at three sizes |
| What Tor gives and does not give, and how to get Tor running | PARTIAL | at "New identity" without Tor, Create is disabled and a banner offers "Tor details", whose dialog says to open Tor Browser and leave it open or start the service, then press Check again; nothing says where to get Tor, that Tor Browser must be connected, where the client looks for it, that a change of settings needs a restart, or what Tor does not hide (online status to card holders) |
| Exchanging cards with a stranger, and why the channel matters | PARTIAL | the difference between a card and an invitation is explained; the fingerprint check is offered at acceptance and in the contact's details; nothing says to send a card over a channel the tester trusts or that a card can be substituted |
| Messaging, identity switching, blocking, deletion, message states | VERIFIED in the harness | fixtures, scenarios A to G (window code on a software renderer, in-memory Tor) |
| Files: drop or typed path, save, progress, restoring after a restart, hostile and long names | VERIFIED in the harness | scenarios J to L, fixtures; no drag and drop on real hardware recorded; a typed path in quotes, as Explorer's "Copy as path" gives it, is not stripped; a file chooser of the system is DEFERRED (P8-7) |
| A start that fails (no OpenGL 2.1, no GL context) | MISSING | `eframe::run_native` failing gives exit code 1 and nothing else; on Windows no console; a panic inside the GL stack is not covered either |
| Help, version, diagnostics a tester can send | MISSING | the version is not shown while it is `0.0.0`; no "copy diagnostics" |
| Updating | MISSING | nothing says that replacing the program keeps the data, or to close Monolith first |
| Closing while transfers run | PARTIAL | the warning counts undelivered messages and an unsaved session, not transfers |
| Keyboard focus, order, Enter and Escape, shortcuts | VERIFIED by harness tests | the message list by keyboard is DEFERRED (BL16) |
| Screen readers | UNVERIFIED | AccessKit roles and names in component tests; never tried with NVDA, Narrator or Orca (BL14) |
| Contrast, themes, sizes 1180x760 to 760x520 | VERIFIED by tests and fixtures | five of eight light avatar colours below 4.5:1; no high-contrast theme; zoom not kept; text size of the OS ignored |
| Bidi and shaping | DEFERRED | BL15 |
| A real window on real hardware | PARTIAL | one Windows window photographed and idle; no real Linux window, GPU, IME, HiDPI, drag and drop or clipboard run recorded |

### 3.6 Documents that are stale

- `SECURITY.md`: "Monolith is in its design phase. No release exists."
- `STORAGE.md` 2: "Logs go to standard error or the journal"; `ARCHITECTURE.md` 6 names `tracing`; there is no logging.
- `TOR_INTEGRATION.md`: "Tested with: none yet in this repository"; the private network runs Tor 0.4.9.13.
- `DESKTOP.md`: the idle figures and the 256 MiB preparation times are those of Phase 6, not of 15.5.
- `packaging/README.md`: "The archive runs from the user's home on both" Tails and Whonix, while the Tails profile matches only `/usr/bin/monolith` and the archive's binary is `monolith-desktop`.
- The user READMEs say nothing of file transfer, updating, or that the profile is the only copy of the identities.

## 4. Release threat assessment

| Scenario | What holds today | What is missing | Proposed control |
| --- | --- | --- | --- |
| A malicious installer or update | no installer, no updater, no network request outside Tor; the binary needs no root and no service | nothing tells a tester an archive is the owner's | a release manifest (names, sizes, SHA-256 of the exact bytes distributed, version, commit) signed offline with a minisign key the owner holds, a key for the beta only |
| A compromised download mirror, repository or account | crates from crates.io only, `--locked`, CI actions pinned by SHA | the checksum travels with the archive; a public key published only beside it travels the same way | the key's fingerprint given to each tester by the owner on a second channel; archives obtained only from the owner, never passed on between testers |
| A tampered build that shows the right version | - | the version and commit in About prove nothing | say so: the check is the manifest, before the first run |
| An unsigned Windows binary | - | SmartScreen and antivirus warnings; no Authenticode; Windows testers have no minisign | label the beta unsigned; give Windows testers either a pinned minisign build or the SHA-256 from the owner's message and `Get-FileHash`; Authenticode needs a certificate (owner decision) |
| Identity data deleted on uninstall or by a "damaged" message | uninstall never deletes data; data live outside the program folder | the Windows README's "delete the data folder"; the damaged message's "move the folder elsewhere and start again"; no warning that the profile is the only copy | warnings at creation, in Settings > Data, in the READMEs and in the damaged message; a distinct message for a newer profile |
| A restored backup | the vault refuses nothing it can open | P3 open: a lower epoch after a restore; two running copies of one onion service; an old copy opens with the old passphrase; rollback not detected | no restore procedure in the beta until P3 is decided, or a strict rule: restore only after the original is gone, on one machine, expecting contacts to need new cards |
| Unsafe filesystem locations, shared machines | local paths enforced; Unix modes and owner enforced; reparse points refused on Windows | Windows ACLs unchecked; `--data-dir` may point at a shared place; the Tor cookie's permissions are not checked on Windows; cards saved to a synced Downloads | the beta assumes one Windows account per tester and a local profile; warn when saving a card; an ACL check is a separate decision (D9) |
| Network requests escaping Tor | static bans, loopback-only endpoints, SOCKS proof, T-NET-1 on the CLI | the packaged desktop binary is not traced; nothing on Windows | trace the packaged client under strace on Linux with a driven profile; on Windows, a run with all outbound traffic of the binary blocked by a firewall rule and the denials logged, plus a packet capture with the machine otherwise idle (the Windows DNS Client service resolves on behalf of a process, so a per-process trace misses DNS) |
| Misleading security indicators | the "Tor" indicator names Tor, not the contact; "Verified" is the user's own mark; Delivered only from the runtime | first run does not say what Tor does not hide; nothing names card substitution; nothing says Tor use itself is visible on a monitored network | texts at first run, at "Add contact" and in the READMEs |
| A malicious contact in the beta | the hostile-sender tests of Phases 7 and 8; a sender's words kept apart from the client's | testers may not know that a contact is untrusted | say so in the tester guide; no new surface |
| Sensitive data in logs, dumps, swap | no logs; fixed error sentences; redacted Debug | core dumps on Linux; Windows error reports, page file, hibernation; panic payloads | disable core dumps on Linux; if a panic hook is added, it prints a fixed sentence only; testers told to use full-disk encryption and not to attach dumps |
| Data leaving through "copy diagnostics" | - | a new path for data out of the client | enums only, no names, cards, counts or Debug text; shown to the tester before copying; the clipboard and its history named; a scan test with a fault that adds a name |
| Incompatible upgrades | a newer store is refused and left unchanged; an older one is migrated on write | "damaged" for a newer profile; no downgrade; no test across two built versions | a failure of its own for a newer profile; a test that opens a profile written by the previous build, kept as a fixture; release notes that say whether a beta may change a format |
| Loss of the signing key | - | testers hold the old key | a beta key separate from any later release key; the revocation procedure in `docs/RELEASE.md` |

## 5. Critical blockers for a controlled beta

1. CI was red (section 2); fixed by P9-B0, green on this branch and on
   `main` (sections 13.6 and 14).
2. Testers cannot tell an archive is the owner's, nor which build they run
   (no signed manifest or key channel, version `0.0.0`, version not shown).
3. Nothing has run on a clean machine; the Linux archive needs the glibc
   of Debian 13.
4. A start that fails says nothing.
5. Loss of an identity is irreversible and the documents invite it (the
   Windows README, the damaged message), with no warning that the profile
   is the only copy; a backup cannot be offered safely while P3 is open.
6. First run does not say how to get Tor running on the platform, nor
   what Tor does and does not hide; the card exchange is not explained.
7. A newer profile is called damaged.
8. The packaged desktop binary has no network-isolation evidence, on
   Linux or Windows.
9. No way for a tester to say which build and state they have, or to
   reach a security contact.
10. A lock or version change selects every mutation fault under the rules
    of `mutation/affected.py`; without an exception approved in advance,
    P9-B0 alone would ask for a full campaign (section 8).

## 6. Proposed workstreams

Efforts are engineer-days of implementation and their own tests, not
counting the owner's reviews. Each workstream keeps the GUI a presentation
and command layer with no I/O on the window thread, uses the UI kit, and
changes no protocol, Tor semantics, storage format or transfer rule.

### P9-B0 CI repair

- Scope: `cargo update -p rustix@1.1.5 --precise 1.1.6`, then
  `cargo tree -d` and `cargo deny check` again (another advisory may have
  appeared). The alternative, an ignore with its reason, is not proposed:
  a patch release fixes it.
- Prerequisites: D1, and the mutation exception D15.
- Affected: `Cargo.lock`; `docs/DEPENDENCIES.md` where it names 1.1.5.
- Security: a patch release of the dependency that does the profile's
  file operations on Unix.
- Gates: deny, audit, `cargo check --locked` on 1.95.0 and 1.85.1, the
  tests of `monolith-storage` and the CI run on the branch.
- Effort: 0.5. Exclusions: no other update.
- Approved on 2026-11-11 with D1 and D15, limited to `Cargo.lock` and
  this file; done and published on this branch, and its lockfile taken
  to `main` with the owner's approval (sections 13 and 14).

### P9-S Early clean-machine smoke

- Scope: before any product change, run the existing archives once on a
  clean Windows 10, Windows 11 and Debian 12 (the archive of Debian 12
  built in the container) with a real Tor Browser or `tor`: unpack,
  start, create a profile, Tor found, a message between two machines.
  Problems found here (glibc, OpenGL, SmartScreen, antivirus, Tor paths)
  may change P9-D and P9-E.
- Effort: 0.5 to 1, with machines or VMs the owner provides.

### P9-F Trust model, versions and updates (a design document)

- Scope: `docs/RELEASE.md`: the beta version scheme (`0.1.0-beta.N`, the
  commit in the binary), the signed manifest as the only authority, the
  beta key and its custody, the fingerprint channel to testers, how a
  Windows tester verifies, manual update (close Monolith, replace the
  program, data stay), whether a beta may change a format, how a newer
  profile is refused, key loss and revocation, offline and Tor-only
  verification. No updater, no network request.
- Prerequisites: D2, D3.
- Effort: 2. Exclusions: any automatic update.

### P9-D First run and identity lifecycle

- Scope: platform steps at "Tor details" (where to get Tor, Tor Browser
  connected and left open, where the client looks, that a change needs a
  restart) and Settings reachable before a profile exists; at first run,
  what Tor does and does not hide; at "Add contact", how to exchange cards
  over a trusted channel and why, and the fingerprint; loss warnings at
  creation, in Settings > Data, in the damaged message and the READMEs; a
  failure of its own for a profile of a newer format; a design note on
  backup and P3 before any backup text (D4, D16); the passphrase change
  only if D5 says so.
- Prerequisites: D4, D5, D6, D16.
- Affected: `monolith-ui` (onboarding, dialogs, settings, common),
  `monolith-ui-model` (failure), `monolith-desktop` (convert),
  `monolith-runtime` and `monolith-core` (the newer-format error, and the
  passphrase change if chosen), `STORAGE.md`, `DESKTOP.md`, the READMEs.
- Security: the passphrase change, if chosen, runs the existing vault
  rules through the runtime for the first time; it must keep the old file
  until the new one is durable, remove the temporary copy, and say that
  older copies still open with the old passphrase; it needs its own design
  review.
- Gates: view and text tests at three sizes, fixtures; the newer-format
  test against a profile written by `phase-8-complete`; for the
  passphrase change, a runtime test across a restart and a crash at each
  step under the existing crash model, and targeted faults.
- Effort: 6 to 9 without the passphrase change, 9 to 13 with it.
  Exclusions: no export format, no key escrow, no cloud backup, no
  rotation in the client unless D7.

### P9-G Operational privacy and network evidence

- Scope: T-NET-1 extended to the packaged desktop binary on Linux, under
  strace in a namespace with only loopback, a display on a Unix socket
  (Xvfb with software rendering) and a driven profile (a contact, a dial
  to an unreachable onion, a card import, a file offer), with `bind` and
  `sendmmsg` added to the traced calls and any connect to the system
  resolver's socket treated as a failure; the Windows evidence of section
  4; core dumps disabled on Linux at start, before any thread
  (`RLIMIT_CORE` 0 and `PR_SET_DUMPABLE` 0 through the safe functions of
  `rustix`, which becomes a normal Unix dependency of `monolith-desktop`);
  a decision on a panic hook (a fixed sentence only, or none); the stale
  documents of 3.6; D9.
- Affected: `tests/network/`, `monolith-desktop` (start, `Cargo.toml`),
  `DEPENDENCIES.md`, docs.
- Security: dumpable 0 also stops a same-user ptrace and makes
  `/proc/self` owned by root; check that nothing of the window stack reads
  it; the Linux README says a debugger cannot attach. `RLIMIT_CORE` 0
  does not bind a core pattern that pipes to a handler; dumpable 0 does.
- Gates: the desktop trace passes; a test that the settings are in
  force; a forced crash leaves no core on Debian 12 with
  `systemd-coredump`; the Windows firewall-deny run recorded.
- Effort: 5 to 6 (plus 2 for an ACL check if chosen).

### P9-E Desktop experience and accessibility

- Scope: the version and commit in About; "copy diagnostics" if D8 says
  so, built from enums only (version, commit, OS family, Tor check result,
  the last failure kind), shown before it is copied, offered also on the
  failure screens and in the Tor dialog, with a design note that it is
  not a log; a visible message when the window cannot start (D14); the
  close warning covers transfers; a save-dialog line that a received file
  can be harmful; a warning when saving a card that it holds an
  invitation; typed paths in quotes accepted; an accessibility statement
  for the beta; zoom kept in `desktop.conf` if cheap.
- Prerequisites: D8, D14; the version scheme of P9-F.
- Affected: `monolith-desktop` (start, build id), `monolith-ui`
  (settings, dialogs, app), `monolith-ui-kit` only if a component is
  missing, fixtures.
- Security: diagnostics hold nothing a peer sent, nothing that identifies
  a contact, and no counts; a test scans them, and a fault that adds a
  name must make it fail.
- Gates: view tests and fixtures; the diagnostics scan and its fault; the
  start failure tried in a VM without OpenGL 2.1 (manual).
- Effort: 6 to 9; 4 to 5 if the Windows start message is dropped (D14).
  Exclusions: the file chooser of the system (P8-7, D10), screen-reader
  certification, bidi, high contrast, the keyboard message list (BL14 to
  BL17).

### P9-B Distribution and provenance (Windows and Linux)

- Scope: the beta version in `Cargo.toml` and the commit in the binary;
  the manifest and its signing by the owner (the key never in the
  repository or CI); signed tags; the Linux archive built on Debian 12 in
  the container with the pinned toolchain; a CI job that builds both
  archives and the unsigned manifest as artifacts (the first Windows
  runner, a commit of its own); two Linux builds compared at the same
  path; a deterministic Windows zip and `cargo auditable` and an SBOM only
  if D2 asks for them; uninstall and update texts and an `uninstall.sh`
  that removes only what `install.sh` installed and prints the data
  paths; the clean-machine smoke on the platforms of D12.
- Prerequisites: D2, D3, D12; the document of P9-F; the beta key made by
  the owner.
- Affected: `Cargo.toml` (version), `packaging/`, `.github/workflows/`,
  `SECURITY.md`, `README.md`, `ARCHITECTURE.md` 11, `docs/RELEASE.md`.
- Security: nothing claims an unsigned binary is authenticated; the
  manifest is the trust anchor and is verified before the first run.
- Gates: two Linux builds identical; the manifest verifies on a clean
  machine and a changed archive fails; the smoke list.
- Effort: 8 to 11 (plus 5 or more for Authenticode if chosen).
  Exclusions: installers (BL21) unless D2, an updater, Tor bundling.

### P9-C Tails and Whonix

- Scope for the beta: say plainly that they are not supported in the
  beta; correct `packaging/README.md`. The experiments of
  `PLATFORM_TAILS.md` 3.4 and the two-Workstation test of Whonix stay
  later work (BL22), unless the owner wants them now (D11).
- Effort: 0.5 (documents); Tails 4 to 6 and Whonix 5 to 8 if chosen, with
  real VMs and a possibly negative result for ADD_ONION on Tails.

### P9-H Controlled beta acceptance

- Scope: a runbook of the thirteen workflows of the brief, with negative
  cases (Tor Browser closed and reopened, a wrong cookie path, a full or
  missing drive, an update over a running client, a quoted path) and
  expected outcomes in plain words; a one-page tester guide (what to
  test, how to report, what not to send); evidence (screenshots,
  checklists, the diagnostics text); run first by the owner on clean
  machines over the public Tor network, then by testers the owner
  chooses; optionally a short NVDA trial on Windows.
- Gates: every workflow passes or has a recorded, accepted limit.
- Effort: 5 to 6 (writing, one internal pass, a second pass after fixes).

### P9-I Independent review

- One static, read-only round of five reviewers on one commit (release
  and distribution, privacy and network isolation, identity and storage
  lifecycle, GUI safety and accessibility, build and update integrity),
  as the convergence rules ask; findings with severity, evidence,
  invariant, disposition.
- Effort: 1 to 2 of coordination; fixes counted where they fall.

### P9-J Final verification and beta gate

- Scope: the matrix of section 8 on one frozen candidate, the bounded
  mutation manifest, the report, the completion record in
  `DESIGN_QUESTIONS.md` and the outputs on `verification-<commit>`, the
  owner's decision.
- Effort: 3 to 4, plus the approved mutation run.

## 7. Minimum beta scope

Needed for a controlled beta: P9-B0; P9-S; P9-F; P9-D without a backup
procedure and, by the plan's proposal, without the passphrase change
(texts, Tor steps, warnings, the newer-format failure); P9-G; P9-E with
the version, the warnings and quoted paths (diagnostics and the Windows
start message by D8 and D14); P9-B without installers, Authenticode,
SBOM or a deterministic Windows zip; P9-C as a statement; P9-H; P9-I;
P9-J. About 35 to 50 engineer-days, with the owner's time for the key,
the machines and the beta itself on top.

Later, or for a public release: installers and Authenticode (BL21); an
updater; the file chooser of the system (P8-7); Tails and Whonix
(BL22); screen readers, bidi, the keyboard message list, unread counts
(BL14 to BL17); rotation from the client; a backup action in the client,
after P3; Windows ACL checks; SBOM and reproducible Windows builds; high
contrast.

## 8. Verification strategy and costs

- Ordinary tests first: fmt, clippy (both feature sets), the workspace
  tests with 1.95.0 and 1.85.1 on Linux in CI and on Windows by a recorded
  manual run on the frozen candidate (there is no Windows CI), the current
  stable once at the final gate; deny and audit.
- Security boundaries tested directly: the newer-format refusal against a
  profile of the previous build; the diagnostics scan; the desktop under
  strace; the core-dump settings; the Windows firewall-deny run; the
  passphrase change under the crash model if chosen.
- Mutation: targeted faults only where a fault can be written and a test
  bears on it: the newer-format failure, the diagnostics content, the
  passphrase change if chosen. The core-dump settings and the network
  trace are covered by their tests, not by faults. Under the rules of
  `mutation/affected.py` a change to `Cargo.lock` or `Cargo.toml` (P9-B0,
  the version, the desktop's `rustix` dependency) selects every fault; the
  plan asks now for an exception like those of Phase 8 (D15), so that
  those changes run only the faults their code can reach. The owner
  granted it for P9-B0 only (section 13.4), which also says why every
  later selection in this phase selects every fault. A second exception,
  D18, covers the fuzz lockfile and the minimum of `rustix` (section
  15.4). At the final gate, one bounded run with an exact manifest and
  its cost, for the owner to approve; the manifest names D15 and D18 apart
  from the production changes of the phase. Documents and test tooling
  trigger no run.
- Not repeated: the private Tor network test runs once at the final gate
  if the runtime or the Tor crate changed, else not; the fuzz targets run
  only for a new untrusted input (none is planned).
- Manual: the clean-machine smoke and the beta runbook, with screenshots,
  on the platforms of D12; the Windows network evidence.
- Records: results in `mutation/results-*.json`, the completion record in
  `DESIGN_QUESTIONS.md`, the outputs on `verification-<commit>`, as in
  Phase 8.
- Cost at the final gate: about one day of machine time without mutation,
  plus the approved mutation run (about 15 minutes a fault with two
  workers), provided D15 is granted.

## 9. Decisions asked of the owner

- D1. CI: update `rustix` to 1.1.6 (proposed) or ignore the advisory.
  Decided on 2026-11-11: the update (section 13); done, on this branch
  and on `main`.
- D2. Beta artifacts: portable archives with a signed manifest
  (proposed), or an installer now; a deterministic Windows zip and an
  SBOM now or later (later proposed).
- D3. Signing: a minisign key for the beta, made and held offline by the
  owner, its fingerprint given to each tester on a second channel
  (proposed); Authenticode for the beta or not (it needs a certificate).
- D4. Backup in the beta: none offered, with loss warnings (proposed),
  or a documented procedure after D16.
- D5. Passphrase change in the beta: no (proposed, see section 12), or
  yes after its own design review; a minimum length or a nudge, and
  which.
- D6. Whether a beta may change a storage format, and how testers are
  told.
- D7. Rotation of the transport key from the client in the beta, or
  stated as unavailable (proposed).
- D8. The version shown (proposed, needed); "copy diagnostics" in the
  beta (the reviewers disagree, section 12).
- D9. Windows ACLs: documented, with a one-account-per-tester rule
  (proposed), or checked (a Windows API binding, a decision on
  `unsafe_code`).
- D10. The file chooser of the system: still deferred for the beta
  (proposed), with quoted paths accepted and "Copy as path" documented.
- D11. Tails and Whonix: out of the beta (proposed) or tried by hand now.
- D12. Platforms of the beta: Windows 10 and 11, Debian 12 and an Ubuntu
  LTS (proposed).
- D13. The order of the workstreams (section 10).
- D14. A message when the window cannot start on Windows: a new
  dependency that wraps the system message box (a dependency decision
  under `unsafe_code = "forbid"`), or the README and the exit code only
  (proposed for the beta); on Linux a line on stderr.
- D15. An exception to the selector for P9-B0, the version and the
  desktop's `rustix` dependency: only the faults their code can reach run
  again (proposed), instead of every fault. Granted on 2026-11-11 for
  P9-B0 only, with no mutation run (section 13.4); still asked for the
  version and the desktop's `rustix` dependency.
- D16. P3: left open, with no restore offered in the beta (proposed), or
  a design review of the epoch after a restore now.
- D17. A private channel for vulnerability reports: turn on the private
  vulnerability reporting of the repository host, which `SECURITY.md`
  already names (proposed), or publish an address and a key now (section
  14.3).
- D18. An exception to the selector for updating `fuzz/Cargo.lock` to
  `rustix` 1.1.6 and `tokio` 1.53.3 and raising the minimum of `rustix`
  in `Cargo.toml` to 1.1.6: no mutation run. Granted on 2026-11-11 with
  the change, apart from D15 (section 15.4).

## 10. Proposed order

P9-B0 (with D15), P9-S, P9-F, P9-D, P9-G, P9-E, P9-B, P9-C, P9-H, P9-I,
P9-J. Loss of identities comes first among the product changes, then
network evidence, then failures a user can read, then distribution, as
section 1 asks. The trust model (P9-F) and the owner's key come before
the texts of P9-D and P9-E because every tester document names how to
verify an archive. The early smoke (P9-S) comes before product changes
because what it finds may change them.

## 11. Work items

| ID | Item | Status |
| --- | --- | --- |
| P9-A | Readiness audit, threat assessment, this plan, independent review | done; D1 decided, D15 granted for P9-B0 |
| P9-B0 | CI repair | done; published as `a278e7e` (CI run 43069041967, green); its lockfile on `main` as `88359d4` (CI run 43080006592, green) |
| Maintenance | `fuzz/Cargo.lock` to `rustix` 1.1.6 and `tokio` 1.53.3; `rustix` 1.1.6 required by `Cargo.toml` | done (section 15); D18 granted; to be published only with the owner's approval |
| P9-S | Early clean-machine smoke | proposed; not started |
| P9-F | Trust model, versions and updates | proposed |
| P9-D | First run and identity lifecycle | proposed |
| P9-G | Operational privacy and network evidence | proposed |
| P9-E | Desktop experience and accessibility | proposed |
| P9-B | Distribution and provenance | proposed |
| P9-C | Tails and Whonix statement | proposed |
| P9-H | Controlled beta acceptance | proposed |
| P9-I | Independent review | proposed |
| P9-J | Final verification and beta gate | proposed |

## 12. Independent review of the first draft

Three reviewers, each blind to the others, read the first draft against
the code on 2026-11-11: security and privacy, product and beta
readiness, engineering and verification. Their factual corrections are
taken in above: the Tor dialog already says to open Tor Browser; files
are verified in the harness only; the profile folder is shown in
Settings > Data; `rustix` is a development dependency of the desktop, so
core-dump settings change its manifest; a change of `Cargo.lock` or
`Cargo.toml` selects every fault; there is no Windows CI; three hazards of
a backup, not two, and P3 open; the Windows DNS Client service hides DNS
from a per-process trace; a native message box has no safe binding in the
tree; the efforts were about 20 percent short.

Where they disagree, with each other or with the plan:

- Passphrase change. The product review: defer it, it is the riskiest
  change and of little value without a recovery path. The security
  review: it needs its own design review (an old copy keeps opening with
  the old passphrase; the temporary copy must go). The plan first had it
  in P9-D; it now proposes no (D5).
- Copy diagnostics. The product review: needed for feedback, offered also
  on failure screens. The security review: drop it from the minimum for a
  beta of a few invited testers, or keep it only with enums, no counts and
  a design note. Left to the owner (D8).
- Backup. The product review: show the data folder with a warning, at
  least. The security review: no restore procedure until P3 is decided.
  Both are taken: warnings now, no procedure (D4, D16).
- Determinism. The product review: a deterministic Windows zip and the
  two-build comparison are not needed for invited testers. The
  engineering review: keep the Linux comparison, at the same path. The
  plan keeps the Linux comparison and defers the Windows zip (D2).
- Order. The product review: a clean-machine smoke right after P9-B0. The
  engineering review: the trust model and the version before P9-E. The
  security review: the key and its channel before any tester text. All
  three are taken in section 10.
- Platforms. The product and engineering reviews: Debian 13 duplicates
  the build host; drop it from the smoke list. Taken (D12).

## 13. P9-B0: rustix 1.1.6

Approved by the owner on 2026-11-11: D1, the update, and D15 for this
update only. The commit changes `Cargo.lock` and this file, nothing else.

### 13.1 The change

RUSTSEC-2026-0486 (GHSA-jh2m-fhpj-3vr7, unsound): `getxattr`,
`lgetxattr`, `fgetxattr`, `listxattr`, `llistxattr` and `flistxattr` of
`rustix::fs`, from 1.0.0 to 1.1.5, mark as initialized bytes the system
call never wrote when they are given a zero-length buffer. 1.1.6 fixes
them; the 0.38 series is not affected.

`cargo update -p rustix@1.1.5 --precise 1.1.6`; cargo 1.95.0 and cargo
1.85.1 write the same file. The difference in `Cargo.lock`:

| Entry | Before | After |
| --- | --- | --- |
| `rustix` 1 | 1.1.5, checksum `891efababe418670775f199f0d233d84843c227a0949a883ce15b37c78d6629d` | 1.1.6, checksum `e5e76742e5294153d4dde1207dbec5bdd81eef6fabdc0d2118babadabfd858aa` |
| The 16 packages that depend on it | name `rustix 1.1.5` | name `rustix 1.1.6` |
| `windows-sys` of `rustix`, `errno` 0.3.14, `tempfile` 3.27.0 and `winapi-util` 0.1.11 | 0.52.0 | 0.59.0 |

- 445 packages before and after; none added or removed; no other version
  or checksum changed. `Cargo.toml`, the toolchain and the Cargo
  configuration are unchanged.
- The checksum of 1.1.6 is the one of the crates.io index and of the
  downloaded crate. 1.1.6 is not yanked, is the newest release of
  `rustix`, and declares `rust-version` 1.65.
- The 16 packages: `async-io`, `async-process`, `async-signal`,
  `calloop` 0.14, `calloop-wayland-source` 0.4, `gethostname`,
  `monolith-desktop` (a development dependency, on Unix),
  `monolith-storage` (on Unix), `polling`, `smithay-client-toolkit` 0.20,
  `tempfile`, `wayland-backend`, `wayland-client`, `wayland-cursor`,
  `x11rb`, `zbus`.
- `windows-sys`: the four packages keep their requirements (`>=0.52,
  <0.62`; `winapi-util` `>=0.48, <=0.61`), and 0.59.0 was already in the
  file; cargo resolves them to it when `rustix` moves. None of the four
  is built for Windows: the dependency tree for `x86_64-pc-windows-msvc`,
  with every feature and normal, build and development edges, is the same
  before and after. `windows-sys` 0.52.0 stays, for `glutin`,
  `glutin_egl_sys` and `winit`, as `deny.toml` says.
- The tree for `x86_64-unknown-linux-gnu` differs only in the version of
  `rustix`, with the same features (`alloc`, `default`, `event`, `fs`,
  `net`, `pipe`, `process`, `shm`, `std`, `system`, `time`). `use-libc` is
  not among them, so Linux on x86_64 uses the raw system-call back end of
  `rustix`.

### 13.2 What 1.1.6 changes

The sources of the two releases differ in 26 files of `src`, one of them
new (about 1700 lines of diff). Read against what this tree builds:

- The fix: the six functions above. Monolith calls none of them, and none
  of the 14 crates.io packages that depend on `rustix` 1 has one of their
  names in its sources.
- The raw back end, built on Linux: the padding fields of `Stat`,
  `StatFs` and `Statx` become a `repr(transparent)` wrapper of
  `MaybeUninit`, with the same layout; the `StatVfsMountFlags` constants
  take the `ST_*` values instead of the `MS_*` ones (`ST_RELATIME`
  differs), and Monolith reads only `f_bavail` and `f_frsize` of
  `fstatvfs`; `chown`, `fchown`, `getgroups`, `setuid`, `setgid` and
  `setgroups` change only on 32-bit x86, arm and sparc; the level of the
  IPv6 multicast-hops option is corrected.
- The ancillary data of `recvmsg`, with which the Wayland back end
  receives descriptors: a message length is clamped to the buffer, and
  descriptors are rounded down to whole ones. Linux already reports the
  truncated length, so a well-formed message reads the same.
- `TIOCSCTTY` passes a null argument; `sendfile` is offered on every Linux
  kernel target; documentation.
- Not built here: the libc back end (`rustix` is not built on Windows,
  and Linux uses the raw back end), and the `termios`, `thread` and
  `runtime` modules (their features are off).
- The functions Monolith calls (`open`, `openat`, `mkdir`, `fsync`,
  `flock`, `fstat`, `fstatvfs`, `statat`, `getuid`, `renameat`,
  `renameat_with`, `linkat`, `unlinkat`, `fallocate`, `ftruncate`, `Dir`;
  `mkfifoat` and `inotify` in the desktop's tests) have no change in the
  raw back end other than the padding type.

### 13.3 Checks

Linux: WSL Debian 13 (kernel 6.18, glibc 2.41, 4 CPUs), a fresh clone of
`02d5bde` with the new `Cargo.lock` (SHA-256
`a7580372190016ee274f8de2c2efd8dcd878b8942cb72c75f84d434ba4bdffac`) and a
fresh target directory. The jobs of `.github/workflows/ci.yml`, with its
flags:

| Job | Toolchain | Result |
| --- | --- | --- |
| fmt | 1.95.0 | pass |
| clippy, `-D warnings` | 1.95.0 | pass |
| test | 1.95.0 | 1345 passed, 0 failed, 3 ignored |
| doc, `-D warnings` | 1.95.0 | pass |
| msrv: check `--all-targets` | 1.85.1 | pass |
| msrv: test | 1.85.1 | 1345 passed, 0 failed, 3 ignored |
| stable: test | 1.92.0, the stable of the distro | 1345 passed, 0 failed, 3 ignored |
| stable: test | 1.99.1, on an export of the same tree in the Debian 12 image of the Phase 8 verification (glibc 2.36) | 1345 passed, 0 failed, 3 ignored |
| fuzz-build | nightly 1.101.0 (2026-10-10) | pass; its own lockfile, unchanged |
| deny | cargo-deny 0.20.2 | advisories, bans, licenses, sources ok |
| audit | cargo-audit 0.22.2 | no advisory, no warning |

Beyond `ci.yml`, on the same clone: clippy and the tests with
`monolith-ui/fixtures` (the configuration of the Phase 8 verification:
pass; 1444 passed, 0 failed, 3 ignored, as in Phase 8), `cargo tree -d`,
and T-NET-1 (`tests/network/fail-closed.sh`, pass), whose CLI keeps its
profile through `rustix`. In every Linux run `monolith-storage` passed
its 71 tests (among them the private directory, its lock, refused links
and blobs on disk) and `monolith-desktop` its 45 (among them a pipe or a
device offered as a card, refused without waiting).

Before the update, `cargo deny check` failed on RUSTSEC-2026-0486 and
`cargo audit` reported it as an allowed warning; after it, neither
reports anything.

Windows 11, in this checkout with the new `Cargo.lock`, with 1.95.0:
clippy with `-D warnings` passes and the tests give 1359 passed,
0 failed, 3 ignored. `rustix` is not built there (13.1); this run shows
only that the Windows build still passes. There is no Windows CI.

Not run before the push: CI on GitHub (13.6). Not run: the private Tor
network test (neither the runtime nor the Tor crate changed, and `rustix`
is not on the path of Tor traffic); the packages; mutation (13.4).

### 13.4 D15: no mutation run

Under `mutation/affected.py` a change of `Cargo.lock` selects every fault
(720 in `mutation/README.md`). The owner granted an exception for this
update only, and no mutation run: one patch release of one dependency;
the affected functions are not used; no source of Monolith changes; the
ordinary suite ran (13.3). The exception holds only because the lockfile
changed as 13.1 says.

It is a risk-based exception. It does not show that the binaries are the
same (the Linux builds link `rustix` 1.1.6, whose code differs as 13.2
says) or that the change has no effect on behaviour. `mutation/affected.py`,
`mutation/faults.py`, the results of earlier runs and the verification
outputs are unchanged.

The selector compares each fault's evidence commit with the candidate.
This change stays in that comparison for every fault whose evidence is
older, so every later selection in Phase 9 selects every fault, whatever
else changed. The manifest of the final gate has to name this exception
again, or the owner decides otherwise then.

### 13.5 Left

- `fuzz/Cargo.lock`, the lockfile of the separate fuzz project, still
  had `rustix` 1.1.5 (RUSTSEC-2026-0486) and `tokio` 1.53.1
  (RUSTSEC-2026-0386, unsound; the workspace took 1.53.3 in Phase 8).
  `ci.yml` builds it with `--locked`, but neither deny nor audit reads it;
  it is in no product. Assessed on 2026-11-11 and proposed to the owner:
  `cargo update -p rustix@1.1.5 --precise 1.1.6` and
  `cargo update -p tokio@1.53.1 --precise 1.53.3` in `fuzz/` change four
  lines each (version and checksum, as in `Cargo.lock`) and nothing else;
  `fuzz/Cargo.toml` already allows both. In a scratch copy the fuzz build
  of `ci.yml` passes with them. Only the `contact_store` target builds a
  runtime (current thread, without the I/O and time drivers). Under
  `mutation/affected.py` a change in `fuzz/` selects no fault. Approved
  and done the same day (section 15).
- `docs/DEPENDENCIES.md` named `rustix` 1.1.5 in sections 3 and 4;
  corrected on 2026-11-11, keeping 1.1.5 as the version of the verified
  build of Phase 8.
- `Cargo.toml` kept `rustix = "1.1.5"`, a minimum that 1.1.6 meets.
  Raising it to 1.1.6 would keep a resolve without the lockfile from
  choosing the unsound release; it is a manifest change, not made. The
  crates of the fuzz project inherit this requirement through the
  workspace, so raising it makes the present `fuzz/Cargo.lock` fail
  `--locked`: the two would have to change in one commit. Proposed to the
  owner together with the fuzz update; approved and done with it
  (section 15).

### 13.6 Publication

On 2026-11-11 the owner authorized publishing this branch:
`git push -u origin phase-9-beta-readiness`, an ordinary push of
`02d5bde` and `a278e7e`, checked with `git ls-remote`; no other reference
moved. CI run 43069041967 (workflow `ci`, push, `a278e7e`): the nine jobs
pass.

| Job | Result |
| --- | --- |
| fmt, clippy, doc | pass |
| test (1.95.0), msrv (1.85.1), stable (1.99.1) | 1345 passed, 0 failed, 3 ignored each |
| fuzz-build | pass, with `rustix` 1.1.5 from `fuzz/Cargo.lock` |
| deny | advisories, bans, licenses, sources ok |
| audit | 1458 advisories, 445 crates, nothing reported |

`ci.yml` has no network-isolation job; T-NET-1 ran only here (13.3). A
green run on a development branch is not a release.

The owner then authorized publishing `627f9a7`, the record of this
publication and of the maintenance of `main`: an ordinary fast-forward
push of this branch from `a278e7e`, checked with `git ls-remote`; `main`
and every other reference stayed where they were. CI run 43100233404
(`627f9a7`): the nine jobs pass, with the results of the table above.

## 14. Maintenance of `main`

Approved by the owner on 2026-11-11, apart from Phase 9 and without
merging this branch. Each change is one commit, pushed to `main` as an
ordinary fast-forward after checking that `main` had not moved. No tag
moved and the Phase 8 branches are unchanged; the earlier runs of
`phase-8-complete` stay as they were.

### 14.1 The lockfile of P9-B0

`88359d4` ("Backport rustix security update to main"), parent `9882291`,
changes `Cargo.lock` only: the blob of `a278e7e` (SHA-256 `a7580372...`),
restored from that commit rather than cherry-picked, so that this file
stays off `main`. Before the push, on Linux (WSL Debian 13, a fresh clone
and target, the flags of `ci.yml`): deny and audit report nothing; fmt
and clippy pass; the tests pass with 1.95.0 and with 1.85.1 (1345 passed,
0 failed, 3 ignored each), and so does the check with 1.85.1. CI run
43080006592: the nine jobs pass, with the results of 13.6. D15 covers it
as it covers P9-B0; no mutation ran. `main` now has the sources verified
in Phase 8 with another lockfile, so its build is not the one Phase 8
verified.

### 14.2 The README

`f813528` ("Improve the public README and clarify project limitations"),
parent `88359d4`, changes `README.md` only. It puts the status in a table
of Phases 3 to 8 with their tags and records, and states the limits as
the code and the documents show them: "Start without saving" keeps the
profile in memory, but the operating system and Tor may still write to
disk; there is no offline delivery, and a queued message is sent when
both sides run; there is no telemetry or update check; Monolith does not
bundle or start Tor. It separates the platforms built for, those
verified, releases (none; platforms not decided) and the Tails and Whonix
drafts. A Linux build made on Debian 13 needs glibc 2.39 (the symbol
versions of a build of this tree), so it does not start on Debian 12 or
Ubuntu 22.04. It lists the nine jobs of CI and the checks outside it, and
has one badge, the CI of `main`, served by GitHub. Every relative link
and heading anchor was checked against the files. CI run 43086736623:
the nine jobs pass, with the results of 13.6.

This branch still has the README of `9882291` (14.4).

### 14.3 The security contact

`SECURITY.md` tells reporters to use the private vulnerability reporting
of the repository host. The GitHub API reports that function off for this
repository (2026-11-11), so a reporter has no private channel; the README
says so and promises none. Proposed (D17): the owner turns the function
on in the repository's settings, which makes `SECURITY.md` true as
written, with no new address; an address and a key for encrypted reports
can follow with the trust model (P9-F). Nothing was changed.

### 14.4 Reconciling `main` and this branch

The same lockfile is now in two commits, `88359d4` on `main` and
`a278e7e` here, and `main` has a README this branch lacks. Earlier phases
closed with a fast-forward of `main` to the phase branch; that is no
longer possible. Proposed: one ordinary merge of `main` into this branch,
no rebase, before the first workstream that edits `README.md` (P9-B,
P9-C) or at the latest before the final gate, so that the gate verifies
the tree `main` will receive. `Cargo.lock` is the same on both sides, so
the merge brings only `README.md`, which selects no fault under
`mutation/affected.py`. Not done: it needs the owner.

## 15. The fuzz lockfile and the minimum of `rustix`

Approved by the owner on 2026-11-11, with an exception of its own to the
mutation selector (D18, 15.4). The commit changes `Cargo.toml`,
`fuzz/Cargo.lock`, `docs/DEPENDENCIES.md` (the two advisory paragraphs
that named the fuzz lockfile) and this file. No source of Monolith
changes.

### 15.1 The change

Checked first against the advisory database of 2026-11-11 (commit
`fd6f3981`, 1458 advisories): `cargo audit --file fuzz/Cargo.lock`
reported RUSTSEC-2026-0486 for `rustix` 1.1.5 (patched in 1.1.6 and later)
and RUSTSEC-2026-0386 for `tokio` 1.53.1 (patched from 1.51.6 below 1.52.0
and from 1.53.3), both unsound, as allowed warnings.

    cargo update --manifest-path fuzz/Cargo.toml -p rustix@1.1.5 --precise 1.1.6
    cargo update --manifest-path fuzz/Cargo.toml -p tokio@1.53.1 --precise 1.53.3

and, in `[workspace.dependencies]` of `Cargo.toml`, the requirement of
`rustix` from `"1.1.5"` to `"1.1.6"`, nothing else on the line.

| File | Before | After |
| --- | --- | --- |
| `fuzz/Cargo.lock`, `rustix` | 1.1.5, checksum `891efababe41...` | 1.1.6, checksum `e5e76742e529...` |
| `fuzz/Cargo.lock`, `tokio` | 1.53.1, checksum `202caea871b6...` | 1.53.3, checksum `ce3335fa7184...` |
| `Cargo.toml`, `rustix` | `version = "1.1.5"` | `version = "1.1.6"` |

- Eight lines of `fuzz/Cargo.lock` and one of `Cargo.toml`, as 13.5
  predicted. No other version, checksum, package or dependency list of the
  fuzz lockfile changed (79 packages before and after); the two new
  checksums are those of `Cargo.lock`. `fuzz/Cargo.toml` allowed both
  versions and is unchanged. Cargo 1.95.0 made the change; nightly gives
  the same file (13.5).
- `Cargo.lock` is unchanged (SHA-256 `a7580372...`): it held `rustix`
  1.1.6 already. Both workspaces resolve with `--locked` after the change.
  The minimum goes with the fuzz lockfile because the crates of the fuzz
  project inherit it from the workspace, and with the old fuzz lockfile
  `--locked` fails (13.5).
- In the fuzz project `rustix` 1.1.6 comes through `monolith-storage`, and
  `tokio` 1.53.3 through `monolith-core`, `monolith-tor` and the targets
  themselves; no crate is there in two versions.
- The requirement of `tokio`, in `Cargo.toml` and in `fuzz/Cargo.toml`,
  stays 1.53.1, which RUSTSEC-2026-0386 covers; the lockfiles hold 1.53.3.
  Raising it is not part of this change; it would be the same kind of
  change as the one for `rustix`.

### 15.2 Checks

Linux: WSL Debian 13 (kernel 6.18, glibc 2.41, 4 CPUs), a fresh clone of
`627f9a7` with the change applied, the target directory of the checks of
14.1 (the same sources and `Cargo.lock`; cargo rebuilt what the change
asked), the flags of `ci.yml`.

| Check | Toolchain | Result |
| --- | --- | --- |
| `cargo deny check` | cargo-deny 0.20.2 | advisories, bans, licenses, sources ok |
| `cargo audit` | cargo-audit 0.22.2 | 1458 advisories, 445 crates, nothing reported |
| `cargo audit --file fuzz/Cargo.lock` | cargo-audit 0.22.2 | 79 crates, nothing reported (before: the two warnings) |
| `cargo metadata --locked`, both workspaces | 1.95.0, nightly | ok |
| fmt | 1.95.0 | pass |
| clippy, `-D warnings` | 1.95.0 | pass |
| test | 1.95.0 | 1345 passed, 0 failed, 3 ignored |
| doc, `-D warnings` | 1.95.0 | pass |
| msrv: check `--all-targets` | 1.85.1 | pass |
| msrv: test | 1.85.1 | 1345 passed, 0 failed, 3 ignored |
| fuzz-build of `ci.yml` (`--cfg fuzzing`, `--bins --locked`) | nightly 1.101.0 (2026-10-10) | pass, no warning |
| every target built with `cargo fuzz build` (release, AddressSanitizer) | nightly 1.101.0 | 22 of 22, with the old lockfile and with the new |

`monolith-storage` passed its 71 tests and `monolith-desktop` its 45, as
in 13.3. The new minimum does not raise the MSRV: `rustix` 1.1.6
declares 1.65.

Windows 11, in this checkout with the change, 1.95.0: clippy with
`-D warnings` passes; the tests give 1359 passed, 0 failed, 3 ignored, as
in 13.3. Neither the fuzz project nor `rustix` is built there.

The fuzz targets, with the two builds (old and new lockfile) side by side:

- Replay: each of the 22 targets ran its committed seeds, and for the
  eleven that have one the local working corpus (copied, read only),
  through both builds with `-runs=0`. All ended without a failure. The
  edges covered are equal for 17 targets. For the others the counts move
  between runs of the same binary as well: the hash maps of the standard
  library take a random seed in each process, and so does
  `IdentityId::generate`.
- `contact_store`, the only target with a runtime (current thread,
  without the I/O and time drivers), is where `tokio` 1.53.2 and 1.53.3
  could show: its fixes are in the semaphore, `mpsc`, the blocking pool,
  the timer and `JoinSet`. 262 inputs, made by a 60-second run of the old
  build with a fixed seed, ran through both builds with the functions they
  cover listed (`-print_coverage`, symbolized). The functions of the
  Monolith crates and the edges they cover are the same in both builds,
  apart from the random generation of identity ids. The differences are in
  `tokio`: `Semaphore::acquire` is a function of its own in 1.53.3 (it
  now checks `MAX_PERMITS`, a fix of 1.53.2) and `Acquire::poll` covers
  one more edge, and the target's closure, into which that code is
  inlined, differs by one edge.
- Smoke: each target ran from its seeds for 10 seconds (`contact_store`
  60) with a fixed seed and the limits of `fuzz/README.md`, on a
  scratch corpus thrown away afterwards. No failure, no crash.
- No target source, seed or committed corpus changed. The fuzz evidence of
  Phase 8 stays what it was: runs with `rustix` 1.1.5 and `tokio` 1.53.1.

Not run: the CI of this commit (it is not pushed); `stable` 1.99.1; the
private Tor network test (no runtime or Tor code changed); the packages;
a fuzzing campaign; mutation (15.4).

### 15.3 What the minimum does

A resolve without a lockfile, or `cargo update`, already chose the newest
`rustix`, so the old minimum mattered only to an explicit
`--precise 1.1.5`, `-Z minimal-versions`, or a lockfile written before
1.1.6. The new minimum makes the unsound release unreachable for the
workspace and its fuzz project, and records which release is safe. No
crate of the workspace is published.

### 15.4 D18: no mutation run

Under `mutation/affected.py` a change of `fuzz/Cargo.lock` selects no
fault (`fuzz/` apart from `fuzz/seeds/`), and a change of `Cargo.toml`
selects every fault. The owner granted an exception for this change only,
separate from D15, and no mutation run: no source of Monolith changes;
the application already resolved `rustix` 1.1.6 and `tokio` 1.53.3, so
its lockfile and its builds are the same; the fuzz lockfile serves
development only; the ordinary tests, the security checks and the fuzz
builds ran (15.2). A full run of the historical faults would not give
evidence in proportion to this change.

It is a risk-based exception. It does not show that the binaries are the
same (the fuzz binaries now link `tokio` 1.53.3 and `rustix` 1.1.6,
whose code differs) or that the change has no effect on behaviour. It
holds only for the diff of 15.1; a larger diff would need the owner
again. `mutation/affected.py`, `mutation/faults.py`, the results of
earlier runs and the verification outputs are unchanged.

At the final gate the manifest names D15 and D18 as exceptions for
dependency maintenance, apart from the production changes of the phase,
whose faults are chosen by the code they reach (section 8).
