# Status of the Phase 7 work

Written on 2026-10-18, when Phase 7 started, so that the work can be
picked up on another machine. This file describes work in progress on the
branch `phase-7-security-hardening`. It is removed when Phase 7 is closed;
it does not belong on `main`.

Branch: `phase-7-security-hardening`, created from `main` at `20a6779`.
That is `phase-6-complete` (`92de84a`, whose production code is that of
`68ac655`, the commit on which Phase 6 was verified) and two documentation
commits after it: the wording of the README and section 13.4 of
`docs/DESIGN_QUESTIONS.md`. The production code of Phase 5 was verified on
`91bf22c`.

## 1. What Phase 7 is

Security and protocol hardening: the backlog of `docs/DESIGN_QUESTIONS.md`
sections 13.3 and 13.4, and the deferred items of earlier phases that
those sections name. No new user-facing feature: no file transfer, no
attachments, no new transport or identity model. Phases 5 and 6 stay
closed; nothing here redesigns them. Section 14 of
`docs/DESIGN_QUESTIONS.md` holds the decisions (P7-1 and on).

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history is rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 6 are a frozen baseline: no old test is weakened, no fault
  of an earlier list (B, S, CAP, Q, CS, MR, UI and the rest) is
  renumbered or weakened. A pattern that moves keeps its defect exactly.
- Each bug: reproduce it, a regression test that fails before the fix,
  the smallest safe fix, a look at sibling patterns, a mutation fault
  where useful. Deterministic schedules and paused time over stress.
- The Phase 7 mutation list is `H1` and on.
- Independent reviews are made with Opus, each blind to the others.
- The phase is complete only after its final verification on one frozen
  commit and the final report.

## 2. Backlog inventory

Every item of 13.3 and 13.4, and the items of 12.8 that 13.3 refers to
or that belong with them. Nothing leaves this list except as fixed,
shown not to be a defect, or deferred with the owner's approval.

Classes: C correctness, S security, P protocol compliance, V privacy,
R resource hardening, U UX correctness, D documentation, T test and
tooling. Severity as the reviews gave it; for 13.3, which gave none,
as judged here.

| ID | Source | Sev. | Class | Component | Item |
| --- | --- | --- | --- | --- | --- |
| BL1 | 13.3 | medium | S, R | core `runtime.rs` (MessageAck and ChatMessage handling) | A session's `activity` is refreshed before the message is checked, so a contact can keep an idle session open up to `MAX_SESSION_LIFETIME` |
| BL2 | 13.3 | medium | S, V | desktop `files.rs`; `DESKTOP.md` 10 | Windows card file: mapped network drives, links in intermediate directories, online-only cloud files, no file ID check; the save directory likewise |
| BL3 | 13.3 | medium | P | core `runtime.rs` (catch-all arm); `PROTOCOL.md` 8.7, 13.1; S13 | File messages and Profile are ignored; a file message for an unknown id must be a violation |
| BL4 | 13.3 | low | C | core `runtime.rs` (drive), `sessions.rs` (`probe_ended`); `duplicate.rs` | The Ping of a duplicate-session probe is sent just before the store wait of `next_to_send`; `probe_ended` does not check the credentials again |
| BL5 | 13.3 | low | S, U | ui-model `present.rs` (`is_invisible`) | Tag characters and other format characters are not drawn as visible marks |
| BL6 | 13.3 | low | D | ui `app.rs` | A stale comment |
| BL7 | 13.3 | low | T | ui-kit `theme.rs:662` | The literal Rust 1.99.0 warns about (`float_literal_f32_fallback`) |
| BL8 | 13.3 | low | D | ADR 0009; core `sessions.rs` | "A fresh random nonce": the code falls back to a fixed nonce if the random source fails |
| BL9 | 13.3 | low | D | ADR 0009; core `runtime.rs` (registration) | Two branches of the duplicate rule that production never reaches |
| BL10 | 13.3 | low | R, D | `RESOURCE_LIMITS.md` 12; storage `messages.rs` | Each stored message rewrites the whole store: write amplification by a hostile contact |
| BL11 | 13.3 | low | D | `DESIGN_QUESTIONS.md` 12.3 | Ended queues also count toward `MAX_QUEUED_MESSAGE_BYTES` |
| BL12 | 13.3 | low | D | `DESIGN_QUESTIONS.md` 12.6 (R5-28) | The store called "a small file" |
| BL13 | 13.3 | medium | R | core runtime, Tor accept path; `RESOURCE_LIMITS.md` 6 | None of the seven rates is enforced |
| BL14 | 13.3 (Phase 6) | - | U, T | desktop | Tests with screen readers |
| BL15 | 13.3 (Phase 6) | - | U | desktop, ADR 0010 | Shaping and bidi |
| BL16 | 13.3 (Phase 6) | - | U | desktop | A message list reachable by keyboard |
| BL17 | 13.3 (Phase 6) | - | U | desktop, core | Keeping unread counts and declines |
| BL18 | 13.3 (Phase 6) | - | S | core contact store | Blocking a key without a record |
| BL19 | 13.3 (Phase 6) | - | R | tor `system.rs` (SOCKS proof) | The cost of the SOCKS proof for each failed dial |
| BL20 | 13.3 (Phase 6) | - | S | tor, desktop settings | Reading the Tor cookie by name |
| BL21 | 13.3 (Phase 6) | - | T | packaging | Installers and signing |
| BL22 | 13.3 (Phase 6) | - | T | desktop on Tails and Whonix | The client on Tails and Whonix, tested by hand |
| BL23 | 13.3, 12.8 | low | T | core `tests/runtime.rs` (`Peer::start`) | The publication wait is not bounded, so CS29 is caught by the timeout |
| BL24 | 13.4 | medium | P, C | core `runtime.rs` (scheduler, `connect`), `dialplan.rs`; `PROTOCOL.md` 4, 11.4 | Only the first card of the dial plan is dialed: no fallback to the active card during a rotation |
| BL25 | 13.4 | medium | S, U | ui `requests.rs`, `dialogs.rs`; P6-5 | A sender's display name can close its quotes and read as the client's own words |
| BL26 | 13.4 | medium | C | storage `message_records.rs`, core `identity.rs` (open); `RESOURCE_LIMITS.md` 5 | Lowering a limit of the store makes a store above it unreadable |
| BL27 | 13.4 | low | U | core `runtime.rs` (`send_message`, `send_error`), `api.rs` | Shutdown reported as `UnknownIdentity`; a failed random source or a send cut short as `Storage` |
| BL28 | 13.4 | low | C, D | core `runtime.rs` (`end_record`), `identity.rs` (`queue_message`); 12.5 | The peer is named before the detached task; the record is not looked at again after the vault wait; 12.5 stale |
| BL29 | 13.4 | low | C | core `runtime.rs` (scheduler) | `DeliveryDeferred` with a retry that never comes, after a block or a deletion |
| BL30 | 13.4 | low | C | core `runtime.rs` (Pong handling) | A Pong after the probe's deadline ends a live preferred session as a violation |
| BL31 | 13.4 | low | C | core `supervisor.rs`, `runtime.rs` (publish) | `ServiceState::NoKey` is never reported |
| BL32 | 13.4 | low | C | core `runtime.rs`, `supervisor.rs` (`jitter`); `RESOURCE_LIMITS.md` 7 | The jitter is not uniform |
| BL33 | 13.4 | low | V | core `messages.rs`, storage `message_records.rs`; `CRYPTOGRAPHY.md` 8 | Decoded message texts are not erased from memory |
| BL34 | 13.4 | low | D | ADR 0008 "Writing", `STORAGE.md` 3.5 | Self-contradiction on removing a lone `messages.new` |
| BL35 | 13.4 | low | D | storage `messages.rs` (`recreate`) | Stale comment |
| BL36 | 13.4 | low | U | runtime `lib.rs` (`OpenError`), storage `lib.rs` (`StorageError` Display) | A store that does not authenticate is told as a wrong passphrase |
| BL37 | 13.4 | low | D, R | `RESOURCE_LIMITS.md` 11 | The memory of a write of the store is probably understated |
| BL38 | 13.4 | low | U | core `runtime.rs` (`tor_check`), tor `system.rs` | "Check Tor" says Tor is not running when only the control endpoint fails |
| BL39 | 13.4 | low | C | tor `control/mod.rs` | The cookie file Tor names is compared case-sensitively on Windows |
| BL40 | 13.4 | low | R | storage `disk_windows.rs` (rename), `vault.rs` | The rename of `vault.new` is tried once |
| BL41 | 13.4 | low | S | desktop `files.rs` | A card path that names a device is opened before it is refused |
| BL42 | 13.4 | low | V, S | desktop `config.rs`, `lib.rs`; tor `auth.rs` | Cookie path, `--data-dir` and `--config` on a network share are accepted |
| BL43 | 13.4 | low | U | desktop `window.rs` | The saved size of the window is divided by the zoom |
| BL44 | 13.4 | low | U | desktop `window.rs`, `places.rs`; `DESKTOP.md` 3 | The window thread looks at the disk |
| BL45 | 13.4 | low | U | ui-model `model.rs`; ADR 0011 | A contact of another identity accepting shows a toast over the identity shown |
| BL46 | 13.4 | low | U | desktop `lib.rs`, ui `onboarding.rs`; `PLATFORM_TAILS.md` 5 | On Tails a refused `--data-dir` is explained wrongly, a refused `--config` not at all |
| BL47 | 13.4 | low | U, R | ui-model `present.rs`, ui-kit `bubble.rs` | `MAX_RENDERED_UNBROKEN_RUN` is not applied |
| BL48 | 13.4 | low | U | ui-kit `theme.rs`, `avatar.rs`, `tests/contrast.rs` | White initials below 4.5 to 1 on five light avatar colours |
| BL49 | 13.4 | low | U, D | ui-model `present.rs`, ui `dialogs.rs` | `MAX_NAME_BYTES` documented wrong and unused; the name fields' limit |
| BL50 | 13.4 | - | U | ui composer, egui-winit | Not established: Enter during the composition of an input method |
| BL51 | 12.8 | low | D | ADR 0009 | A record without a dialable card is `DeliveryDeferred` with `NoEndpoint`, never dialed |
| BL52 | 12.8 | low | D, T | core `persist.rs`, `messages.rs`, `sessions.rs` | `cargo doc` with warnings denied finds broken intra-doc links |
| BL53 | 12.8 | - | T | toolchain | rust-lang/rust#164250: build and test again with the stable compiler that fixes it |

Proposed for deferral, waiting for the owner's approval (features or
platform work, not hardening): BL14, BL15, BL16, BL17, BL18, BL20, BL21,
BL22; BL53 until a fixed stable exists. BL19 is either measured and
accepted or deferred.

## 3. Progress

For each item: the decision, the commit, the regression test, the
mutation fault, the document changed, the final status.

| ID | Decision | Commit | Test | Fault | Document | Status |
| --- | --- | --- | --- | --- | --- | --- |
| BL1 to BL53 | | | | | | open |

## 4. Order of the work

A, correctness and security: BL24, BL25, BL2 with BL41 and BL42, BL3,
BL26. B, runtime: BL4 with BL30, BL1, BL13 with BL10, BL39, BL23, BL28,
BL29, BL31, BL32, BL27, BL40. C, interface: BL5, BL38, BL43, BL48, BL44,
BL45, BL46, BL47, BL49, BL50, BL36. D, documents: BL6 to BL12, BL33 to
BL35, BL37, BL51, BL52, BL7.
