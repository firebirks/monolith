# Status of the Phase 5 work

Written on 2026-10-11, while Phase 5 is in progress, so that the work can
be picked up on another machine. This file describes work in progress on
the branch `phase-5-messaging-runtime`. It is removed when Phase 5 is
closed; it does not belong on `main`.

Branch: `phase-5-messaging-runtime`, created from `phase-4-complete`
(`3c013f5`), whose production code is that of `970316f`, the commit on
which Phase 4 was verified. The faults CS1 to CS123 and the regression
tests of Phase 4 were all present at the start.

## 1. What Phase 5 is

The messaging runtime, without a user interface: a message is queued
durably before it is sent, stored durably by the receiver before it is
acknowledged, sent again while its delivery is uncertain, taken in once,
and recorded delivered only by its acknowledgement; restarts of either
side, of Tor, and a lost session do not lose or duplicate it. One owner
for the sessions of each identity; dialing only through Tor and only with
something to deliver; work that stops when a contact or an identity is
gone; commands and events for a later interface, with none of the
authority of the core. No wire change, no change of Noise XK.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history of Phases 1
  to 4 is rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phase 4 is a frozen security baseline: no old test is weakened, no
  fault of an earlier list is renumbered.
- The phase is complete only after its final verification on one frozen
  commit and the final report (section 5).

## 2. Decisions taken

`docs/DESIGN_QUESTIONS.md` section 12 lists them (P5-1 to P5-15) with the
open points; the ADRs are `docs/adr/0007-delivery-semantics.md`,
`0008-message-store.md` and `0009-session-manager.md`.

## 3. What is done

| Commit | What |
| --- | --- |
| `06dceb8` | ADRs 0007 to 0009 |
| `2ef330a` | the limits of the message store, the queue and acknowledgements |
| `679713e` | durable identifiers in the vault (payload version 2), the message store file and its records |
| `9db48cd` | the messages of every contact record in the core: the state machine, its durability, reconciliation with the vault, taking in and acknowledging |
| `564e106` | the session manager, the runtime, the command and event interface, `monolith-runtime` |
| `0e67b47` | `dev-runtime`, a development node on the runtime |
| `b20368f` | fuzz targets `message_store` and `message_sequence`, with seeds |
| `7aebaf4` | mutation faults MR1 to MR28 and the tests they need |
| `35428bf` | the fixes of the internal review and of the two independent reviews (R5-1 to R5-19), with their tests and faults MR29 to MR49 |
| `b1c66cc` | the property tests `tests/message_model.rs` and `tests/runtime_model.rs` |
| `2c00010` | steps 13 to 20 of the private Tor network test |
| `01b1be1` | the documents of the phase |
| `f0f738e` | a deterministic test of the durability wait of Phase 4 and fault MR50 (R5-20) |
| `a6fa560` | the fixes of the review of the fixes (R5-21 to R5-27), faults MR51 to MR57 |
| `705009e`, `638573a` | the mutation runner checks each copy before a run: a known fault is caught, and restored it is built again and passes |
| `9d8f1bd` | the documents of the review of the fixes and of the check of the runner |
| `63d91f6` | the mutation runner measures its timeout from the unmodified run (R5-29) |

## 4. What was verified

During development, in the WSL clone of this machine (Debian, Rust
1.95.0), on a build made from scratch: the workspace tests (39 binaries,
775 passed, 2 ignored, on `f0f738e`), clippy with warnings denied and
`cargo fmt`; the property test of the state machine at 512 cases; every
fault of MR1 to MR57 that was added or moved after `7aebaf4` tried
against the tests of its crate and caught by the test written for it. A
dry run of the private Tor network test in a container on `01b1be1`
passed every step, 1 to 20. Twice a test failed because the tool that
mirrors the tree for the Linux build ran a binary still built with a
fault (`docs/DESIGN_QUESTIONS.md` R5-20); the tool was fixed, the builds
were thrown away, and the mutation runner now checks its copies itself.

Before the freeze, on the official runner: on `638573a` the faults of
Phase 4 in `persist.rs` (CS45, CS55, CS70, CS88, CS89, CS92, CS93, CS94,
CS114, CS115, CS123) were all caught, each by tests that failed; then the
unmodified run of a copy did not end within the fixed bound of 20
minutes, and the runner stopped (R5-29). It takes 972 s on this machine
with nothing else running. On `63d91f6`, with the bound measured, a copy
built from scratch passed its check, its unmodified run took 923 s, and
MR50 was caught by the deterministic test of the wait and two other
tests of `persist.rs`. The dry run of the private Tor network test,
repeated on `63d91f6` after the fixes of the review of the fixes, passed
every step in 861 s.

The final verification has not run.

## 5. What is left, in order

1. The final verification on one frozen commit: format, clippy, the tests
   on Rust 1.85.1, 1.95.0 and the current stable, `cargo deny`, `cargo
   audit`, the duplicate dependencies, every fuzz target briefly and the
   affected ones long, the mutation lists of Phases 1 to 5, the fail-closed
   test and the private Tor network test with steps 13 to 20.
2. The final report, here.
