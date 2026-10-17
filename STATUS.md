# Status of the Phase 6 work

Written on 2026-10-14, while Phase 6 is in progress, so that the work can
be picked up on another machine; the final report (section 6) was added on
2026-10-18. This file describes work in progress on the branch
`phase-6-desktop-client`. It is removed when Phase 6 is closed; it does
not belong on `main`.

Branch: `phase-6-desktop-client`, created from `phase-5-complete`
(`3be1254`), whose production code is that of `91bf22c`, the commit on
which Phase 5 was verified. The mutation faults of Phases 1 to 5 (MR1 to
MR57 and CS1 to CS123 among them) and their regression tests were all
present at the start.

## 1. What Phase 6 is

The desktop client: egui and eframe over the runtime of Phase 5, with a
design system of its own, so that a person can use Monolith without the
CLI, on Linux and on Windows. `docs/DESKTOP.md` describes it;
`docs/DESIGN_QUESTIONS.md` section 13 has the brief in short and the
decisions (P6-1 to P6-21); the ADRs are `docs/adr/0010-gui-toolkit-egui.md`,
`0011-ui-security-boundary.md` and `0012-design-system.md`.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history is rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 5 are a frozen baseline: no old test is weakened, no fault
  of an earlier list is renumbered or weakened. CS40 and CS41 point at
  `disk_unix.rs` since the Unix code of the vault directory moved there;
  their text is unchanged.
- The phase is complete only after its final verification on one frozen
  commit and the final report.

## 2. Building

The whole workspace builds and its tests pass on Linux and on Windows
(P6-13). The client: `cargo run --release -p monolith-desktop`; the
pictures of every screen: `cargo run -p monolith-ui --features fixtures
--example screens -- <dir>`; the workflows A to G by script:
`cargo run --release -p monolith-desktop --example scenarios -- <dir>`;
the smoke test of step 21 of the private Tor network test, on the
in-memory Tor: `cargo run --release -p monolith-desktop --example
tor_smoke`; the frame costs: `cargo run --release -p monolith-ui
--features fixtures --example measure`; the archives:
`packaging/README.md`. The mutation
runner and the private Tor network test run on Linux only.

## 3. What is done

- Runtime additions of P6-4 (requests, invitations, names, verification,
  request mode, service state, Tor check, unblocking, whether a contact's
  card is an invitation) with `tests/front_end.rs`.
- Windows: the vault directory (`disk_windows.rs`, `STORAGE.md` 3.8).
- The SOCKS proof of the system Tor backend (`THREAT_MODEL.md` adversary
  O): the desktop dials only once Tor lists its SOCKS endpoint.
- The UI model, the design system (components, fonts, icons, theme), the
  screens, the bridge and the window; Tails places (P6-16).
- Every finding of the four reviews (visual, UX, security and
  architecture, egui) fixed or documented, then checked again by two
  closure reviews on `a316dbf`, whose remaining items were fixed after.
- Tests: the model scenarios of the brief's section 42, property tests of
  the model against a model of the runtime, the components, contrast, the
  screens (source scan, views, smallest window, idle, keyboard), the bridge
  against the runtime, the boundary, the files; two fuzz targets; the
  Phase 6 mutation list UI1 to UI160; the workflows A to G by script; step
  21 of the private Tor network test (two clients driven through their
  window code over Tor A and Tor B, `examples/tor_smoke.rs`); packaging
  for Linux and Windows with third-party notices; the documents.

## 4. What was verified

Before the freeze, by the mutation runner in WSL, every fault of UI1 to
UI160 was caught in targeted runs on the commit where it was added or its
pattern moved: UI1 to UI32 and UI34 first (UI13 and UI27 to UI34 on
`21b3df0`); UI33 in its present form and UI35 to UI49 on `47ae971`; UI50
to UI87 on `a316dbf`; UI3, UI72 and UI88 to UI110 on `94d57d9`; UI14,
UI51, UI57, UI63, UI83, UI87 and UI111 to UI145 on `ea0b12f`; UI146 to
UI160 on `68ac655` itself (2026-10-15, 06:19 to 06:43). Every test of the
workspace passed on Windows on the way (1153 on `ea0b12f`), and so did
the workflows A to G and the Tor smoke test on the in-memory Tor.

The final verification of `68ac655` ran from 2026-10-15 06:44 to
2026-10-18 00:44, in full, and passed. Its results are the final report,
section 6.

## 5. What is left, in order

1. The owner's review of the final report (section 6).
2. On the owner's instruction only: whether the outputs of the
   verification go on a branch of their own, as for Phases 4 and 5; the
   push, the merge into `main` and the tag. Then this file is removed.
3. The follow-up work of section 6, item 35, in the next phase, or before
   the merge if the owner wants it there. A change before the merge needs
   a new frozen commit and the stages of the verification it affects.

## 6. Final report

Written on 2026-10-18, after the final verification of `68ac655`. Each
result comes from an output of that verification, named where it is used,
or from the repository at `68ac655`. The outputs are kept on the machine
that ran it (item 33); they are not in the repository.

### 1. Phase 5 baseline

- Tag `phase-5-complete` on `3be1254`, which is also `main` and
  `origin/main`. Its production code is that of `91bf22c`, the commit on
  which Phase 5 was verified.
- What Phase 6 had to keep: the tests of Phases 1 to 5, and the lists
  `PHASE1` to `PHASE5` of `mutation/faults.py` (45, 99, 71, 123 and 57
  faults, 395 in all) with their outcomes on `91bf22c`: 386 caught and
  nine expected to survive, each documented as equivalent, not observable
  or unreachable (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29). None was
  weakened, renumbered or removed.

### 2. Verified commit

- `68ac655bb82f4d423d46581bdc9a1816f8f87161` ("Say whom an unblock names,
  outline a toast over a dialog, fit the first start in short windows, use
  real cards in the fixtures, and keep short texts whole"), on
  `phase-6-desktop-client`, committed on 2026-10-15 at 06:19 +02:00 and
  frozen at 06:44.
- Every result of items 21 to 27 and 32 was produced on this commit, with
  a clean tree:
  - Windows stage: a detached worktree at `68ac655` with a target
    directory of its own, 2026-10-15 from 06:44 to 07:15;
  - WSL stage: a clone checked out at `68ac655`, 2026-10-15 from 06:44 to
    07:18;
  - container stage: an export of the tree of `68ac655` without `.git`
    (`git archive`), marked with its hash, 2026-10-15 from 11:41 to 12:27;
  - Linux package: built from the WSL clone at `68ac655`, since
    `packaging/linux/build.sh` takes the root of the repository and the
    commit time from git, which the export has not (item 22);
  - mutation lists: copies made by `mutation/run.py` with `git archive`
    from a clone checked out at `68ac655`, each copy checked before its
    first fault (a known fault is caught, and restored it builds again and
    passes), from 2026-10-15 07:58 to 2026-10-17 20:41, and T6 alone on
    2026-10-18 (item 26).
- No file of the repository changed between the freeze and the end of the
  verification. The commit that adds this report changes `STATUS.md`
  only.

### 3. Branch and history

- `phase-6-desktop-client` from `3be1254`: 60 commits up to `68ac655`,
  all with `firebirks <336470935+firebirks@users.noreply.github.com>` as
  author and committer, no trailer (checked in the WSL stage). No history
  was rewritten.
- Not merged and not tagged. Pushed at the owner's request on
  2026-10-14, three times from 21:41 to 21:52, the last up to `6c5cc1e`,
  where `origin/phase-6-desktop-client` still is.

### 4. Crate and module architecture

`docs/ARCHITECTURE.md` and ADR 0011:

- `monolith-ui-model`: the state of the client, with no egui: the model,
  handles, commands and outcomes, failures, the presentation of peer
  text.
- `monolith-ui-kit`: the design system.
- `monolith-ui`: the screens, drawn from the model with kit components.
- `monolith-desktop`: the window, the bridge to the runtime on a thread
  of its own, files, settings, places.
- `monolith-ui-snapshot`: a software renderer for pictures and tests,
  never in a release.

The front end of Phase 5 (`monolith-runtime`) gained the queries and
commands of P6-4. Only `monolith-desktop` depends on the runtime; no UI
crate holds an authority-bearing object of the core (S58).

### 5. egui and eframe

egui and eframe 0.32.3, pinned exactly, with the glow renderer, X11 and
Wayland, and AccessKit 0.19. 0.32.3 is the newest that builds with Rust
1.85 (ADR 0010).

### 6. Design-system architecture

ADR 0012:

- One theme of tokens: colours by role for light and dark, Inter and
  JetBrains Mono, spacing, radii, strokes, shadows, motion with a reduced
  setting, sizes. The theme is installed into egui's style too.
- Components built from egui's painter and responses, each with its
  states, and with its role, label and enabled state for AccessKit.
- A focus ring, shown after keyboard moves only.
- Contrast checked by a test for every text on every surface it is drawn
  on, and for wells and fields against what they sit on.
- A source scan that fails on a stock egui widget, or on a colour, font
  or size of a screen's own.

### 7. Component inventory

In `crates/monolith-ui-kit/src`: avatar, badge, banner and inline error,
message bubble with day separator, button (primary, secondary, ghost,
danger, quiet danger, busy), checkbox, chrome (panels, scroll view with
its own bar, sidebar edge), composer, divider, empty state, fingerprint,
icon button, icons (a Lucide subset), identity switcher, invitation row,
mark and wordmark, menu, message status, modal, QR code, request card,
rows (conversation and contact rows with markers), search field, section
header, segmented control, settings row, spinner, text, text field (secret
and no-history modes), toast, toggle, tooltip, Tor status.

### 8. Visual design decisions

- Neutral surfaces with one accent, at the density of a desktop
  messenger; rounded cards and modals.
- Inter for text, JetBrains Mono for fingerprints and cards; fingerprints
  in rows of seven and six.
- Status said by an icon and a word.
- A sidebar 260 to 420 points wide; one pane below 880 points, for the
  settings too; the window at least 760 by 520.
- Toasts at the top, clear of notices and dialogs.
- A person glyph for anyone without a name the user gave.

### 9. Identity workflow

A profile on disk under a passphrase, or a session that saves nothing.
Identities are made from the switcher or the settings (Tor is needed, and
the client says so with a next step), switched in the switcher with counts
per identity, and deleted after a confirmation. The card of the identity
is one click away. The window title counts what waits.

### 10. Contact workflow

- A card or an invitation card is pasted, or read from a file dropped or
  named: 64 KiB at most, a regular file only, drive paths only on
  Windows.
- The client says whether it was an invitation, and what the other side
  must do after a plain card (P6-20).
- Names are local. Contacts are verified by fingerprint.
- Block, remove and delete messages each ask with a confirmation that
  says what happens. Blocked people are listed and unblocked after a
  question (P6-21).

### 11. Invitation workflow

An invitation is made with a local label and shown as a card with its QR
code, which can be copied or saved; "N of 16 active" is shown. It is
revoked with or without the requests it admitted. The texts are true under
each request mode. After a request accepted through an invitation, a toast
offers to revoke it.

### 12. Messaging workflow

- Enter sends; Shift+Enter makes a new line.
- A draft is cleared only of the text the runtime queued; what is typed
  during a send is kept.
- Pages of 100 messages, up to 2000; long messages collapsed; the list
  virtualised.
- A deferred delivery is said with its reason, with the fact that both
  sides must be open, and with the time of the next attempt.

### 13. Queued and Delivered

Exactly the runtime's status of each message. Delivered comes only from
the runtime's answer for that message, after the contact's durable
acknowledgement (ADR 0007); never from pressing Send, from the command
returning or from a session (S60: UI3, UI5 and UI6, the model tests, the
workflows by script).

### 14. Tor status

Checking Tor, Connecting to Tor, Tor connected, Tor unavailable: in the
top bar and in the narrow list, with details that say what the last check
found and the next step. The desktop dials only once Tor, authenticated by
SAFECOOKIE, lists its SOCKS endpoint (adversary O). The proof is off on
Tails and Whonix and in the CLI.

### 15. Offline behaviour

- Messages stay Queued, with the reason.
- An offline contact is said as "may be closed or offline".
- Tor being down blocks delivery, not composing.
- Closing asks first when a session saves nothing or messages are not
  delivered.
- A failed store is said once, by a banner that asks for a restart.

### 16. Command and event integration

- The window sends commands and takes updates through two bounded
  channels, and never waits.
- Queries and events run on one task, in order. Writes run as tasks of
  their own and are answered through it.
- Events are notifications that make one read of the part they name.
  Missed events make a full read and a Tor check.
- A dying runtime thread wakes the window.

### 17. Stale events and lifecycle

- Handles of one identity name nothing of another.
- Answers are taken only for a command in flight, and only for the
  identity and dialog that asked.
- A contact made again inherits nothing.
- Numbers of requests, invitations and blocked people are never reused,
  and are forgotten once not listed.

S59 (UI1, UI2, UI4, UI7 to UI13, UI17 to UI20); the unblocking faults
UI77 to UI87; the property tests and the `ui_model_sequence` fuzz target.

### 18. UI security boundary

ADR 0011, S58 to S62:

- No authority-bearing core object in the UI crates (a boundary test).
- Peer text is drawn as plain text, with invisible and direction-changing
  characters as visible marks.
- No URL is opened (link opening is removed from every frame). The client
  has no socket, process or network path of its own.
- No secret or message text goes into debug output, egui memory, logs or
  the settings file.
- The passphrase buffer never reallocates, and the input method is off in
  secret fields.
- Card files are opened only as checked regular files.

### 19. Accessibility

- What is done: keyboard use of every screen but the message menu; a
  focus ring; AccessKit roles, names and enabled state; status in words;
  contrast of 7 to 1 (WCAG 2.1) for main text and 4.5 to 1 otherwise, in
  both themes; reduced motion.
- What is not claimed (DESKTOP.md section 9): testing with a screen
  reader; messages out of view in the tree; shaping and bidi; the system
  text size; a high-contrast theme.

### 20. Keyboard

- Tab and Shift+Tab move over controls only, not over a scroll bar, a
  drag area or the sidebar edge.
- Enter, Space and Escape (dialogs, filter, settings); Ctrl+F; Alt+Up and
  Alt+Down; zoom.
- Shortcuts are held while any question is open.

### 21. Performance

Measured with the `measure` example, release build, at 1180 by 760:

- On Windows, after the first frame (2.5 ms; 2.9 ms with the context and
  the fonts), every case
  took at most 1.17 ms a frame (the slowest frame of scrolling 2000
  messages of 16 KB). That includes 2000 messages of 16 KB and a sidebar
  of 500 contacts.
- In the container: at most 0.75 ms.
- A burst of 1000 events was applied in 0.05 ms, and asked for 2 reads.
- The packaged client on Windows, idle on its first screen: 0 ms of CPU
  over 10 s, a working set of 142 MB.
- No frame is drawn when nothing happens (a test over every fixture). The
  Tor pulse runs at 4 Hz, and the next-attempt time is redrawn once a
  minute.

### 22. Packaging

A Linux tar.gz and a Windows zip, built by `packaging/linux/build.sh` and
`packaging/windows/build.ps1`. Each holds a README and a
THIRD-PARTY-NOTICES.txt generated from the dependency graph of its
target: the licence texts, fonts and icons, 168 crates on Windows and
247 on Linux (14 and 15 of them without a licence text of their own,
covered by the standard SPDX texts).

- `monolith-desktop-0.0.0-windows-x86_64.zip`: 4,547,125 bytes, SHA-256
  `a4b5aa04f627dbff2d47f0266170f10c235927cc453085fb59583dcedda56e6f`,
  built in the Windows stage.
- `monolith-desktop-0.0.0-linux-x86_64.tar.gz`: 6,556,104 bytes, SHA-256
  `d7f31d7a64812d7525e265db33fdd2035cc20106489af08c4a51cf3048d02a1d`.
  The package step of the container stage stopped at once with exit code
  128, because the export has no `.git` (item 2). The package was built
  instead from the WSL clone at `68ac655` on 2026-10-15 at 12:35, with
  the same script, and its checksum was checked.

### 23. Tests

Workspace tests, each with `--features monolith-ui/fixtures`:

| Where | Toolchains | Test runs | Passed | Failed | Ignored |
| --- | --- | --- | --- | --- | --- |
| Windows | 1.95.0 and 1.85.1 | 76 | 1165 | 0 | 2 |
| Linux (container) | 1.95.0, 1.85.1 and stable 1.99.0 | 76 | 1167 | 0 | 2 |

- The 76 test runs are 64 test binaries and 12 doctest runs.
- Linux runs seven Unix-only tests and Windows five Windows-only ones (the
  vault directory, card files), so Linux has two more.
- The two ignored tests are
  `a_service_published_again_from_its_key_is_reached_again`, which needs
  the private Tor network and is run by the two-node script, and
  `unlock_times_at_the_candidate_settings`, a measurement.
- On `91bf22c`, 782 tests passed.
- Rust 1.99.0 warns once, `float_literal_f32_fallback`, a lint of
  future incompatibility, at `crates/monolith-ui-kit/src/theme.rs:662`
  (item 35).

Fail-closed T-NET-1 passed: SOCKS available, SOCKS unavailable with
direct Internet, the CLI without Tor, a namespace with loopback only.

The two-node private Tor network test passed every step of Phases 3 to 6
in 790 s, with 63 `ok` lines. It ran with Tor 0.4.9.13, Chutney
`ae3a33c`, and every Monolith process under strace (loopback and Unix
sockets only, no DNS). This run used `RESTART_CYCLES=3`; the run on
`91bf22c` used 15.

Phase 6 added:

- model scenarios, and property tests against a separate model of the
  runtime;
- kit tests (components, contrast, focus, accessibility, chrome, modal,
  toasts);
- screens (views, keyboard, layout, idle, source scan);
- the bridge against the runtime; the boundary; files;
- the `front_end` tests of the runtime; the Tor SOCKS proof;
- the workflows A to G by script, and step 21 of the private Tor network
  test.

### 24. Toolchain matrix

| Check | Where | Toolchain | Result |
| --- | --- | --- | --- |
| `cargo fmt --check` | container, Windows | 1.95.0 | pass |
| clippy, warnings denied, all targets, with and without fixtures | container, Windows | 1.95.0 | pass |
| workspace tests | container, Windows | 1.95.0 | item 23 |
| `cargo check`, all targets | container, Windows | 1.85.1 | pass |
| workspace tests | container, Windows | 1.85.1 | item 23 |
| workspace tests | container | stable 1.99.0 | item 23 |
| `cargo tree -d` | container | 1.95.0 | pass |
| fuzzing | WSL | nightly 1.101.0 | item 25 |
| `cargo deny check` 0.20.2 | WSL | 1.95.0 | advisories, bans, licenses, sources ok |
| `cargo audit` 0.22.2 | WSL | 1.95.0 | 445 crates, no advisory |

- The container is `rust:1.95-bookworm` (Debian 12), the WSL clone is on
  Debian 13.4, and Windows is Windows 11.
- The duplicates of `cargo tree -d` are those of the window stack, which
  `deny.toml` accepts by exact version and `docs/DEPENDENCIES.md` lists,
  and the pair of `rand_core` of the core (0.9.5 through proptest, for
  tests only, and 0.10.1), as on `91bf22c`. None of the window stack is
  cryptographic.
- In the container, clippy and the check on 1.85.1 took under a second:
  they reused the build of the earlier run of the same export (item 33).

### 25. Fuzzing

- `cargo fuzz build` and the fuzz check passed.
- Each of the 20 targets ran for 26 s, with no crash.
- The two targets Phase 6 added ran for 600 s each, with no crash:
  `ui_present` (1,218,306 runs) and `ui_model_sequence` (2,407,157 runs).
- `tor_control_reply` covers the new reply of the listener list.

### 26. Mutation results

`mutation/run.py` ran with 2 workers. The timeout of each list is
measured from its unmodified run: twice the workers times that run, and
at least 20 minutes.

| List | Faults | Caught | Survived | Unmodified run, timeout |
| --- | --- | --- | --- | --- |
| Phase 6 (UI) | 160 | 160 | 0 | 651 s, 2604 s |
| Phase 5 (MR) | 57 | 57 | 0 | 526 s, 2104 s |
| Phase 4 (CS) | 123 | 123 | 0 | 439 s, 1756 s; after the pauses 512 s, 2048 s |
| Phase 3 | 71 | 68 | 3 (Q14, Q18, Q29) | 441 s, 1764 s |
| Phase 2 | 99 | 95 | 4 (S15, S24, CAP2, CAP3) | 452 s, 1808 s; after the interruption 504 s, 2016 s |
| Phase 1 | 45 | 43 | 2 (B3, B4) | 512 s, 2048 s |
| Total | 555 | 546 | 9 | |

- Every outcome is the one `mutation/faults.py` expects. The nine
  survivors are those of `91bf22c`. No fault was invalid.
- Each of the 555 ids has exactly one outcome line in the logs of the run.
- The run was interrupted four times on the same commit. Each time it
  went on with the faults that had not ended; the copies were made again
  and the unmodified run was measured again. A fault in flight was run
  again from the start, and no outcome was counted twice or lost.
  - 2026-10-15 at 11:05: the WSL VM was restarted from outside the
    verification. The Phase 5 list had ended 16 faults, all caught. It
    was run again whole from 12:29, after the container stage; the first
    log is kept apart.
  - 2026-10-15 at 17:21, at the owner's request, after 6 faults of the
    Phase 4 list. Resumed on 2026-10-16 at 19:37.
  - 2026-10-16 at 19:39: paused again while the owner decided on the
    review of item 30. Resumed at 19:41.
  - 2026-10-17 at 13:02: the process that held the run ended, after 58
    faults of the Phase 2 list. Resumed at 13:04.

Caught at the timeout: MR9, MR25, MR26, CS29, CS30, Q1, T6. In each case
the result file names the tests that failed before the timeout, except for
MR25, MR26 and T6:

- MR25 and MR26 turn a loop of the runtime into one that never waits. With
  time paused, a test waits for good. This is the same as on `91bf22c`
  (`docs/DESIGN_QUESTIONS.md` 12.7).
- MR9: `a_failed_attempt_is_not_repeated_at_once`.
- CS29: twelve tests of `crates/monolith-core/tests/front_end.rs`, which
  Phase 6 added and which runs before `tests/runtime.rs`. Among them are
  `a_block_outlives_the_profile_and_a_handle_from_before_names_nothing`,
  `a_tor_check_says_what_the_endpoints_answer` and
  `an_invitation_is_listed_shown_again_and_revoked_by_its_handle`. On
  `91bf22c`, CS29 was caught by the timeout alone (12.8).
- CS30: three tests, two of `tests/supervisor.rs` (among them
  `a_lost_service_is_published_again_under_the_same_name`) and one of
  `tests/front_end.rs`.
- Q1: four tests of `crates/monolith-tor/tests/system.rs`, among them
  `without_a_socks_endpoint_there_is_no_connection_at_all` and
  `a_failure_of_tor_ends_the_proof`.
- T6: it fell in the part of the Phase 2 list that the result file of the
  resumed list does not hold. It was run again alone, with one worker, on
  2026-10-18 from 00:14 to 00:44 (unmodified run 551 s; timeout 1200 s,
  the least the runner allows). It was caught again, and 340 named tests
  failed before the timeout, among them
  `a_body_of_the_wrong_length_is_rejected` and
  `a_card_text_with_one_changed_symbol_is_rejected`. No catch depends on
  the timeout alone, except those of MR25 and MR26.

### 27. Phase 1 to 5 non-regression

- Every earlier test passes (item 23).
- The lists `PHASE1` to `PHASE5` on `68ac655` gave exactly the outcomes of
  `91bf22c`: 395 faults, 386 caught, the same nine survivors (item 26).
- The production changes below the runtime (`git diff 3be1254 68ac655`):
  - `monolith-core`: the queries and commands of P6-4, unblocking and the
    invitation flag among them; `add_contact` returns an `AddedContact`;
  - `monolith-storage`: the Windows vault directory, with the Unix code
    moved to `disk_unix.rs`;
  - `monolith-tor`: the SOCKS proof, off by default;
  - `monolith-cli`: adjusted to both;
  - `monolith-protocol`, `monolith-session` and `monolith-identity`: no
    change.
- The structure test passes. Its only change turns the path separators
  into `/`.

### 28. Visual review findings and fixes

- Review A on `21b3df0`: 24 findings.
- Its closure on `a316dbf`: 13 fixed, 10 partly fixed, 1 open. These were
  fixed after.
- A last review of every picture on `630dc3d`: no blocker and no major.
  Its 11 minors and most nits were fixed in `68ac655`.

### 29. UX review findings and fixes

- Review B: 25 findings, among them a blocker (a plain card failed
  silently). They were fixed: card versus invitation, unblocking,
  closing, delivery wording, Nobody, switcher counts and card, narrow
  settings.
- Its closure on `a316dbf`: 13 fixed, 11 partly fixed, none open. The
  partial ones were fixed after.

### 30. Security review findings and fixes

- Review C: nothing critical or high. A medium, Tor Browser's free SOCKS
  port, was fixed by the SOCKS proof. The lows were fixed.
- Its closure on `a316dbf` found eight new lows, fixed after: re-blocking
  after an unblock said, closing that counts sends in flight, the
  passphrase buffer, the wake on a dying thread, the close on Wayland, the
  draft cut, the keyboard limit documented, the history of the card
  field.
- After the freeze, a review of Phases 5 and 6 was checked against the
  code on 2026-10-16. It found no way around authentication or the
  contact rules, and nothing that changes a result of this verification.
  Its real findings are the first part of item 35. The owner chose to
  close the phase on `68ac655` and to leave them to the next phase.

### 31. egui-specific review

- Review D found a high: every frame rebuilt every message. It was fixed
  by the virtualised list and caches.
- The mediums and lows were fixed: the Tor pulse, undo, modal height,
  disabled controls, Tab stops, selection, reduced motion, toasts,
  discarded frames, closing, the QR code.
- The closure confirmed the fixes.

### 32. Manual scenario results

- Workflows A to G, by script, through the real window code, bridge and
  runtime on the in-memory Tor: passed on Windows (17 pictures) and in the
  container.
- Step 21 of the private Tor network test passed. Two desktop clients, one
  process each, were driven through their window code over Tor A and
  Tor B, with the desktop's Tor configuration (the SOCKS proof on). Tor
  listed each SOCKS endpoint. A added B's invitation from a file and
  wrote; B accepted and answered. Each client showed its message
  Delivered and the other's received. Both ran under strace, with
  loopback and Unix sockets only.
- The Tor smoke test on the in-memory Tor passed on Windows.
- A release build of the client with the `screenshot` feature (never part
  of a package) opened on Windows and on WSLg, in light and in dark, and
  took a picture of its own window. The packaged client itself was run on
  Windows for item 21.

### 33. Screenshots and artifacts

- Pictures of every fixture: 68 fixtures, light and dark, at 1180 by 760,
  820 by 600 and 760 by 520 (408 pictures), from Windows and from the
  container.
- Pictures of the workflows, and of the real window on Windows and on
  WSLg.
- The two packages and their checksums.
- Logs of every stage, every mutation log and result file, and the
  records of the pauses.

All of these are kept on the machine that ran the verification. Whether
they go on a branch of their own is the owner's decision (section 5).

Incidents of the container stage, none of which touched a result:

- The first container was lost when the Docker Desktop engine stopped,
  at 07:52.
- Docker Desktop could not start again until the WSL VM was restarted
  (item 26, 11:05).
- One run lacked the capabilities of the fail-closed test, and was run
  again with `--cap-add SYS_PTRACE --security-opt seccomp=unconfined`.

The results above are those of the complete run, from 11:41 to 12:27.

### 34. Documented limitations

- DESKTOP.md sections 9 and 13:
  - a screen reader is not tested;
  - no shaping or bidi;
  - the message menu needs a pointer;
  - the passphrase is entered by paste when an input method is needed;
  - unread counts are not kept, and declines are not listed;
  - re-blocking after an unblock;
  - the window position is not kept;
  - the permissions of Windows folders;
  - the QR code is not scanned.
- THREAT_MODEL.md adversary O: the 30 s window, and a failed dial ends
  the proof, so the next dial opens a control connection
  (TOR_CONTROL_SURFACE.md).
- Known residuals that stay as documented:
  - egui's copies of text and the clipboard history (ADR 0011, DESKTOP.md
    10);
  - the count in the window title (DESKTOP.md 5);
  - the ancestors of the Windows vault are resolved again, and their ACLs
    are not checked (STORAGE.md 3.8);
  - no bidi (ADR 0010);
  - the deduplication window of 1024 (ADR 0007 and 0008);
  - no revocation of transport keys (ADR 0002, F-R4 and F-R5).

### 35. Deferred work for Phase 7

From the review of Phases 5 and 6 checked after the freeze (item 30).
Code:

1. The `activity` of a session is refreshed before the message is checked:
   in `crates/monolith-core/src/runtime.rs`, for a MessageAck before
   `acknowledged()`, and for a ChatMessage even when it is refused or is
   a duplicate. A contact can keep an idle session open up to
   `MAX_SESSION_LIFETIME` (24 h). The fix is to refresh it only on an
   acknowledgement that names a message in flight, and on a message taken
   in.
2. Reading a card file on Windows (`crates/monolith-desktop/src/files.rs`):
   - mapped network drives pass the local path check;
   - links in intermediate directories are followed;
   - online-only cloud files are read over the network;
   - the check that the opened file is the one looked at compares no file
     ID.
   The fix is to refuse remote drives, reparse points along the path and
   files that are not on the disk, to check the opened handle, and to
   check the save directory the same way (DESKTOP.md section 10).
3. The runtime ignores every file message and Profile (the catch-all arm
   in `runtime.rs`). PROTOCOL.md 13.1 makes a file message for an
   unknown id a violation. A minimal fix without transfers: reject each
   FileOffer at once, remember its id, and treat any other file message
   on an unknown id as a violation.
4. The probe of a duplicate session: the Ping is sent just before the
   store wait of `next_to_send` (R5-28), so a live preferred session can
   lose its 10 s probe. `probe_ended` does not check the credentials
   again. Liveness only.
5. `is_invisible` (`crates/monolith-ui-model/src/present.rs`) misses the
   tag characters (U+E0001, U+E0020 to U+E007F) and other format
   characters.
6. A stale comment in `crates/monolith-ui/src/app.rs`.
7. The literal that Rust 1.99.0 warns about (item 23), made `2.0_f32`.

Documents:

- ADR 0009 says "a fresh random nonce"; the code falls back to a fixed
  nonce if the system's random source fails.
- ADR 0009 describes two branches of the duplicate rule that production
  never reaches.
- RESOURCE_LIMITS.md 12 should name the write amplification a hostile
  contact can cause (each stored message rewrites the whole store).
- DESIGN_QUESTIONS.md 12.3: ended queues also count toward
  `MAX_QUEUED_MESSAGE_BYTES`.
- DESIGN_QUESTIONS.md 12.6 (R5-28) calls the store "a small file".

Rates: none of the seven rates of RESOURCE_LIMITS.md section 6 is
enforced yet. The per-contact rates come first; the rates of the accept
path are hardening.

Already deferred:

- tests with screen readers; shaping and bidi;
- a message list reachable by keyboard;
- keeping unread counts and declines;
- blocking a key without a record;
- the cost of the SOCKS proof for each failed dial;
- reading the Tor cookie by name;
- installers and signing;
- the client on Tails and Whonix, tested by hand;
- bounding the publication wait of `Peer::start` in `tests/runtime.rs`
  (12.8).

### 36. Final verdict

Each completion criterion of the brief holds on `68ac655`:

- normal messaging works end to end without the CLI: items 9 to 12 and
  32;
- the client looks deliberately designed, with no stock egui appearance
  on a primary surface: items 6 to 8 and 28 (the source scan, item 6);
- the multi-identity workflow works: items 9 and 17;
- contacts, requests and invitations work: items 10, 11 and 32;
- messaging works: items 12 and 32;
- Queued and Delivered are those of Phase 5 exactly: item 13;
- offline and Tor-unavailable behaviour is clear: items 14 and 15;
- the runtime stays authoritative: items 13 and 16;
- stale events cannot cross a lifecycle or identity boundary: item 17;
- the client owns no authority-bearing core object, has no network path
  outside the Tor runtime, and leaks no secret or message text through
  its diagnostics: item 18; step 21 under strace, item 32;
- idle use is reasonable: item 21;
- resizing works: item 8, the screens tests of the smallest window (item
  23), the pictures at three sizes (item 33);
- the visual reviews are closed: item 28;
- the accessibility limitations are documented as they are: items 19 and
  34;
- every Phase 1 to 5 regression stays green: item 27;
- every Phase 6 fault behaves as expected, 160 of 160 caught: item 26;
- the final verification is green on one frozen candidate, `68ac655`:
  items 2 and 21 to 27.

The limitations of item 34 are documented and none breaks a requirement
of the brief. The work of item 35 is not needed for this verdict. The
findings of the review checked after the freeze are follow-up work: none
is a way around authentication or a contact rule, and none changes a
result above. Point 2 of item 35 concerns a file the user names on a
network or cloud drive; the client itself opens no connection for it.
The owner chose to close the phase on `68ac655` and to leave these
findings to the next phase.

PHASE 6 COMPLETE
