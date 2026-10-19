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

Deferred with the owner's approval on 2026-10-18 (features or platform
work, not hardening; `docs/DESIGN_QUESTIONS.md` 14.2): BL14 to BL17,
BL18, BL20 (only finding the cookie by the name Tor gives; the checks
of the configured path are BL42), BL21, BL22 (the build, package and
tests of Linux and Windows stay in the verification), BL53 (the final
verification looks at the stable of the day, and if it has the fix,
builds and tests with it). BL19 is measured, and accepted or deferred.

## 3. Progress

For each item: the decision, the commit, the regression test, the
mutation fault, the document changed, the final status.

| ID | Decision | Commit | Test | Fault | Document | Status |
| --- | --- | --- | --- | --- | --- | --- |
| BL24 | fix (P7-1) | see git log | runtime tests `a_successor_that_does_not_answer_is_followed_by_the_active_key` and five more; `link::tests::only_a_card_that_did_not_answer_is_followed_by_the_next` | HR1 to HR4 | ADR 0009, ARCHITECTURE 1.1, DESIGN_QUESTIONS P7-1, mutation/README | fixed |
| BL25 | fix (P7-2) | see git log | `requests::tests::the_name_a_sender_chose_is_in_no_sentence_of_the_card`, views `the_name_a_sender_chose_is_in_no_sentence_of_the_screen_or_its_dialog`, kit `a_name_a_sender_chose_stays_in_the_box_of_their_words`, `prose::tests` | HR5 to HR8 | DESKTOP 6, S15, TEST_PLAN T-UI-7, DESIGN_QUESTIONS P7-2, mutation/README | fixed |
| BL2, BL41, BL42 | fix (P7-3) | see git log | `local::tests`, `files::tests` (junction, symbolic link, offline, device names, a file swapped on Windows), `config::tests`, `tests` of `lib.rs` | HR9 to HR11; HR12 to HR16 (Windows) | DESKTOP 10, DESIGN_QUESTIONS P7-3, mutation/README | fixed; the Windows list waits for the runner on Windows |
| BL3 | fix (P7-4) | see git log | `transfers::tests`, runtime `a_file_offer_is_rejected_at_once_and_what_crosses_it_is_discarded`, `a_file_message_for_a_transfer_never_offered_ends_the_session`, `what_one_session_rejected_is_unknown_to_the_next` | HR17 to HR20 | PROTOCOL 8.7 and 13, S13, DESIGN_QUESTIONS P7-4, mutation/README | fixed |
| BL26 | fix (P7-5), the vault too | see git log | `messages::tests::a_store_above_a_lowered_limit_is_read_and_kept_and_grows_no_further` and two more; `message_records` and `record` tests at and above the ceilings | HR21 to HR23 | RESOURCE_LIMITS intro, ADR 0008, STORAGE 3, DESIGN_QUESTIONS P7-5, mutation/README; MR12, MR13, MR14, MR45 retargeted | fixed |
| BL1 | fix (P7-6) | see git log | `tests/runtime.rs` `a_contact_keeps_no_idle_session_open_with_messages_that_carry_nothing` | HR24, HR25 | ADR 0009, RESOURCE_LIMITS 4, `limits.rs`, DESIGN_QUESTIONS P7-6 | fixed |
| BL4 | fix (P7-6): probe time counts while reading; credentials looked at again | see git log | `a_probed_session_that_answers_while_it_is_not_read_is_kept`, `a_probed_session_that_does_not_answer_loses_to_the_newer_one`; `sessions::tests::a_probed_session_whose_key_was_retired_meanwhile_loses_though_it_answered` | HR26, HR30 | PROTOCOL 8.2, ADR 0009, RESOURCE_LIMITS 4, CRYPTOGRAPHY, TEST_PLAN T-MSG-7 | fixed |
| BL30 | fix (P7-6): one Ping outstanding, a late Pong dropped, `PONG_TIMEOUT` applied | see git log | `a_pong_after_the_probe_was_decided_keeps_the_session`, `a_probe_asked_while_a_ping_is_outstanding_waits_for_its_pong`, `a_ping_not_answered_within_its_timeout_ends_the_session` | HR27 to HR29 | PROTOCOL 8.2, ADR 0009, RESOURCE_LIMITS 4 | fixed |
| BL29 | fix (P7-7), the failed dial too | see git log | `tests/runtime.rs` `a_session_that_ends_with_its_record_announces_no_retry`, `a_dial_that_fails_once_the_vault_failed_announces_no_retry` | HR31, HR32 | ADR 0009, `api.rs`, DESIGN_QUESTIONS P7-7, TEST_PLAN T-MSG-6 | fixed |
| BL28 | fix (P7-7); 12.5 holds again | see git log | `a_block_for_a_record_made_again_before_it_runs_ends_nothing`, `a_send_for_a_record_deleted_while_it_waits_for_the_vault_queues_nothing` | HR33, HR34 | ADR 0009, DESIGN_QUESTIONS 12.5 and P7-7 | fixed |
| BL13 | fix (P7-8): the five rates of contacts; the two of streams and strangers not enforced, with the reason | see git log | `tests/runtime.rs` `a_contact_above_a_rate_is_slowed_and_no_other_contact_is`, `the_rate_of_a_contact_holds_across_its_sessions`, `a_contact_that_opens_sessions_above_its_rate_has_them_closed`, `a_session_that_waits_for_a_token_ends_when_it_is_closed`; `rates::tests` | HR35 to HR43 | RESOURCE_LIMITS 6, 11, 12; PROTOCOL 6 and 14; ADR 0002; THREAT_MODEL B and C; TEST_PLAN T-RES-6; `limits.rs`; DESIGN_QUESTIONS 12.3 and P7-8 | fixed |
| BL10 | documented (P7-9): cost and bound; the rate bounds it; a store that writes what changed left for later | see git log | as BL13 | - | RESOURCE_LIMITS 12, THREAT_MODEL B, DESIGN_QUESTIONS P7-9 | fixed |
| BL40 | fix (P7-10): bounded retry on a sharing violation or a denied access | see git log | `disk_windows::tests::a_rename_waits_a_little_for_a_file_another_process_holds`, `a_rename_of_a_file_held_for_good_fails_in_bounded_time` (Windows) | HR44 (Windows list) | STORAGE 3.8, `disk_windows.rs`, DESIGN_QUESTIONS P7-10 | fixed |
| BL39 | fix (P7-11): ASCII case folded on Windows only; anything else still refused | see git log | `control::tests::a_cookie_file_is_named_without_case_only_where_files_are`, `a_windows_cookie_file_is_named_without_case` (Windows); `tests/system.rs` `a_cookie_file_named_in_another_case_is_that_file_only_on_windows` | HR45; HR46 (Windows list) | THREAT_MODEL, DESIGN_QUESTIONS P7-11 | fixed |
| BL31 | fix (P7-12) | see git log | `tests/runtime.rs` `an_identity_without_a_key_is_reported_so_and_stays_so` | HR47 | DESIGN_QUESTIONS P7-12, TEST_PLAN T-MSG-8 | fixed |
| BL32 | fix (P7-12): rejection sampling, one function | see git log | `supervisor::tests::every_jitter_is_as_likely_as_any_other` | HR48 | RESOURCE_LIMITS 7, DESIGN_QUESTIONS P7-12 | fixed |
| BL27 | fix (P7-12): `SendError::ShuttingDown`, `SendError::Randomness`; desktop says them in place | see git log | `a_send_once_the_runtime_shuts_down_says_so`, `a_send_a_shutdown_cuts_short_says_so`, `runtime::tests::a_failed_send_says_what_failed`, desktop `convert::tests::a_failed_send_is_said_where_it_was_met_unless_the_store_failed` | HR49 to HR52; MR49 retargeted | `api.rs`, DESIGN_QUESTIONS P7-12 | fixed |
| BL23 | fix (P7-13), tooling only | see git log | CS29 now fails `tests/runtime.rs` in about a minute | - | mutation/README, DESIGN_QUESTIONS P7-13 | fixed |
| BL3 (note) | PROTOCOL 8.7 and P7-4: Profile stated as accepted and not used, checked like any other | see git log | as BL3 | - | PROTOCOL 8.7, DESIGN_QUESTIONS P7-4 | fixed |
| BL5 | fix (P7-14): the name rules' invisible set, tags included | see git log | `present::tests::tag_and_other_format_characters_are_shown`; fuzz target `ui_present` | HR53 | DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-4, mutation/README | fixed |
| BL47 | shown not to overflow (P7-14): broken at the width of the bubble, nothing inserted; for the owner to confirm | see git log | kit `a_run_without_a_break_opportunity_is_broken_inside_its_share_of_the_width` | - | RESOURCE_LIMITS 10.1, DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-5 | fixed, to confirm |
| BL48 | fix (P7-14): five light colours darker | see git log | `contrast.rs` covers the initials | HR54 | DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-6 | fixed |
| BL49 | fix (P7-14): 128 bytes and 64 characters, applied to every name field | see git log | kit `the_byte_limit_cuts_typed_and_pasted_text_at_a_character`, views `a_name_field_takes_no_more_than_a_name_may_hold` | HR55 | RESOURCE_LIMITS 10.1, DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-5 and T-UI-7 | fixed |
| the rest | | | | | | open |

## 4. Order of the work

A, correctness and security: BL24, BL25, BL2 with BL41 and BL42, BL3,
BL26. B, runtime: BL4 with BL30, BL1, BL13 with BL10, BL39, BL23, BL28,
BL29, BL31, BL32, BL27, BL40. C, interface: BL5, BL38, BL43, BL48, BL44,
BL45, BL46, BL47, BL49, BL50, BL36. D, documents: BL6 to BL12, BL33 to
BL35, BL37, BL51, BL52, BL7.
