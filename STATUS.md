# Status of the Phase 6 work

Written on 2026-10-14, while Phase 6 is in progress, so that the work can
be picked up on another machine. This file describes work in progress on
the branch `phase-6-desktop-client`. It is removed when Phase 6 is
closed; it does not belong on `main`.

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
decisions (P6-1 to P6-19); the ADRs are `docs/adr/0010-gui-toolkit-egui.md`,
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
  Phase 6 mutation list UI1 to UI147; the workflows A to G by script; step
  21 of the private Tor network test (two clients driven through their
  window code over Tor A and Tor B, `examples/tor_smoke.rs`; first run
  against Tor in the final verification); packaging for Linux and Windows
  with third-party notices; the documents.

## 4. What was verified

Not the final verification yet. On the way:

- Phase 6 mutation list, run by the runner in WSL: UI1 to UI32 and UI34
  caught earlier; UI33 (in its present form), UI35 to UI49 on `47ae971`,
  UI50 to UI87 on `a316dbf`, UI3, UI72 and UI88 to UI110 on `94d57d9`:
  all caught. UI14, UI51, UI57, UI63, UI83, UI87 (patterns moved) and
  UI111 to UI145 running on `ea0b12f`; UI146 and UI147 not run yet.
- Every test of the workspace passes on Windows (1153 on `ea0b12f`).
- Workflows A to G and the Tor smoke test on the in-memory Tor pass.
- Frame costs: under 2.5 ms for every case of the measure example, 2000
  messages of 16 KB included.

## 5. What is left, in order

1. A last visual review of every picture before the freeze.
2. The final verification on one frozen commit (scripts ready): the
   container stage (needs Docker Desktop, with the owner's OK), the WSL
   stage, the Windows stage, the mutation lists of Phases 1 to 6.
3. The final report (36 items), ending in `PHASE 6 COMPLETE` or
   `PHASE 6 NOT COMPLETE`.
