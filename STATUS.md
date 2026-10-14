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
the frame costs: `cargo run --release -p monolith-ui --features fixtures
--example measure`; the archives: `packaging/README.md`. The mutation
runner and the private Tor network test run on Linux only.

## 3. What is done

- Runtime additions of P6-4 (requests, invitations, names, verification,
  request mode, service state, Tor check) with `tests/front_end.rs`.
- Windows: the vault directory (`disk_windows.rs`, `STORAGE.md` 3.8).
- The UI model, the design system (components, fonts, icons, theme), the
  screens, the bridge and the window; Tails places (P6-16).
- Tests: the model scenarios of the brief's section 42, the components,
  contrast, the screens (source scan, smallest window, idle, keyboard),
  the bridge against the runtime, the boundary, the files; two fuzz
  targets (three faults of `present.rs` found and fixed); the Phase 6
  mutation list UI1 to UI34; the workflows A to G by script; packaging for
  Linux and Windows; the documents.

## 4. What was verified

Not the final verification yet. On the way:

- Phase 6 mutation list, on `7e48ce2`: UI1 to UI12 and UI14 to UI26
  caught; UI13 survived, a test was written for it (`1b2971a`); UI13 and
  UI27 to UI34 run again on `21b3df0`.
- Workflows A to G passed against the runtime on the in-memory Tor.
- The release binary on Windows: 0 ms of CPU over 10 s idle, 117 MB.

## 5. What is left, in order

1. The four independent reviews (visual, UX, security and architecture,
   egui), then: reproduce each finding, a regression test before the fix,
   a mutation fault where useful, a review of sibling patterns.
2. The property tests of the UI model (`tests/properties.rs`).
3. The final verification on one frozen commit: fmt, clippy, every test on
   both toolchains and on Windows, deny, audit, every fuzz target, the
   mutation lists of Phases 1 to 6, the fail-closed and private Tor tests,
   the workflows, the packages, the measurements, a visual review.
4. The final report (36 items), ending in `PHASE 6 COMPLETE` or
   `PHASE 6 NOT COMPLETE`.
