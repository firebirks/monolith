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
CLI. `docs/DESIGN_QUESTIONS.md` section 13 has the brief in short and the
decisions (P6-1 to P6-12); the ADRs are `docs/adr/0010-gui-toolkit-egui.md`,
`0011-ui-security-boundary.md` and `0012-design-system.md`.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history is rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 5 are a frozen baseline: no old test is weakened, no fault
  of an earlier list is renumbered or weakened.
- The phase is complete only after its final verification on one frozen
  commit and the final report.

## 2. Building

`monolith-ui-model`, `monolith-ui-kit` and `monolith-ui` depend on no
Monolith crate and build on any platform egui supports.
`monolith-desktop` and everything below `monolith-runtime` build on Linux
only (the storage layer uses Unix file APIs).

## 3. What is done

| Commit | What |
| --- | --- |

## 4. What was verified

Nothing yet.

## 5. What is left, in order

1. The runtime additions of P6-4, with tests.
2. The UI model, the design system, the screens and the bridge.
3. Reviews: internal, visual, UX, security and architecture, egui.
4. The Phase 6 mutation list (UI1 and on), fuzz targets where they add
   value, the documents.
5. The final verification on one frozen commit, and the final report.
