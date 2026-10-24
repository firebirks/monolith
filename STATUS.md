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
- Independent reviews are made each blind to the others.
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
| BL54 | final verification | - | T | `mutation/run.py` | A fault counts as caught by any failing test, also one that fails for another reason, such as its own timeout under load |

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
| BL47 | shown not to overflow (P7-14): broken at the width of the bubble, nothing inserted; the owner confirmed this reading (2026-10-24) | see git log | kit `a_run_without_a_break_opportunity_is_broken_inside_its_share_of_the_width` | - | RESOURCE_LIMITS 10.1, DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-5 | fixed |
| BL48 | fix (P7-14): five light colours darker | see git log | `contrast.rs` covers the initials | HR54 | DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-6 | fixed |
| BL49 | fix (P7-14): 128 bytes and 64 characters, applied to every name field | see git log | kit `the_byte_limit_cuts_typed_and_pasted_text_at_a_character`, views `a_name_field_takes_no_more_than_a_name_may_hold` | HR55 | RESOURCE_LIMITS 10.1, DESIGN_QUESTIONS P7-14, TEST_PLAN T-UI-5 and T-UI-7 | fixed |
| BL38 | fix (P7-15): `SocksStatus::Unproven`, `TorCheck::NoControl` | see git log | tor `tests/system.rs` `with_the_check_on_status_says_the_socks_endpoint_was_not_tried_without_the_control_endpoint`; `runtime::tests::a_tor_check_says_tor_is_not_running_only_where_both_endpoints_were_tried`; views `a_control_port_that_does_not_answer_is_named_where_tor_may_be_running` | HR56, HR57; UI42 retargeted | DESKTOP 5, DESIGN_QUESTIONS P7-15, TEST_PLAN T-UI-7, mutation/README | fixed |
| BL43 | fix (P7-16): the size times the zoom | see git log | `window::tests::the_saved_size_of_the_window_is_its_size_on_the_screen_whatever_the_zoom` | HR58 | DESIGN_QUESTIONS P7-16, TEST_PLAN T-UI-10, mutation/README | fixed |
| BL44 | fix (P7-16): the filter found at the start, the save folder chosen on the thread of the files | see git log | `files::tests::a_card_goes_to_the_downloads_folder_chosen_by_the_thread_of_the_files` | HR59 | DESKTOP 3, DESIGN_QUESTIONS P7-16, TEST_PLAN T-UI-10 | fixed |
| BL45 | fix (P7-17): the notice only for the identity shown | see git log | model `a_contact_of_another_identity_accepting_raises_no_notice_over_the_one_shown` | HR60 | DESIGN_QUESTIONS P7-17, TEST_PLAN T-UI-1, mutation/README | fixed |
| BL46 | fix (P7-17): the first screen says a refused data folder, a notice a refused settings file | see git log | desktop `tests::a_place_named_and_refused_is_said_refused_and_not_replaced`, views `a_data_folder_refused_at_the_start_is_said_so_and_not_as_storage_to_unlock` | HR61, HR62 | PLATFORM_TAILS 5, DESIGN_QUESTIONS P7-17, TEST_PLAN T-UI-7 | fixed |
| BL36 | fix (P7-18): `InstallationError::MessageStoreChanged`, `OpenError::Invalid` | see git log | core `tests/messages.rs` `a_message_store_that_does_not_authenticate_fails_the_installation` (expects the new variant); runtime `tests::a_changed_message_store_is_not_told_as_a_wrong_passphrase` | HR63, HR64 | STORAGE 3.6, DESIGN_QUESTIONS P7-18, mutation/README | fixed |
| BL50 | established and fixed (P7-19): an Enter after a preedit sent the draft; the composer follows the composition | see git log | kit `composer.rs` `enter_while_an_input_method_composes_sends_nothing` | HR65 to HR67 | DESKTOP 8, DESIGN_QUESTIONS P7-19, TEST_PLAN T-UI-5, mutation/README | fixed |
| BL6 | fixed (P7-20): `TorSettings::cookie_file` | see git log | - | - | DESIGN_QUESTIONS P7-20 | fixed |
| BL7 | fixed (P7-20): `2.0_f32`; nightly 1.100 finds no other | see git log | - | - | DESIGN_QUESTIONS P7-20 | fixed |
| BL8 | documented (P7-20) | see git log | - | - | ADR 0009 | fixed |
| BL9 | documented (P7-20) | see git log | - | - | ADR 0009 | fixed |
| BL11 | documented (P7-20) | see git log | - | - | DESIGN_QUESTIONS 12.3 | fixed |
| BL12 | documented (P7-20) | see git log | - | - | DESIGN_QUESTIONS 12.6 | fixed |
| BL33 | documented (P7-20): not erased, said so | see git log | - | - | CRYPTOGRAPHY 8 | fixed |
| BL34 | documented (P7-20) | see git log | - | - | ADR 0008, STORAGE 3.5 | fixed |
| BL35 | fixed (P7-20) | see git log | - | - | `messages.rs` | fixed |
| BL37 | measured and documented (P7-20): 64 to 100 MiB at a write | see git log | measurement outside the repository | - | RESOURCE_LIMITS 11 | fixed |
| BL51 | documented (P7-20) | see git log | - | - | ADR 0009 | fixed |
| BL52 | fixed (P7-20): links made code spans, `cargo doc` in the CI | see git log | `cargo doc` with warnings denied | - | `.github/workflows/ci.yml` | fixed |
| BL19 | measured and accepted (P7-21): about 1.8 ms a failed dial | see git log | throwaway test, release build | - | RESOURCE_LIMITS 11, DESIGN_QUESTIONS P7-21 | accepted |
| BL14 to BL18, BL20 to BL22 | deferred with the owner's approval (2026-10-18) | - | - | - | DESIGN_QUESTIONS 14.2 | deferred |
| BL53 | the final verification looks for a stable compiler with the fix | - | - | - | DESIGN_QUESTIONS 14.2 | at the final verification |
| BL54 | deferred with the owner's approval (2026-10-22); the outcomes are classified by hand, and the store.rs crash tests have a bound of their own | see git log | - | - | mutation/README, DESIGN_QUESTIONS 14.2 | deferred |

## 3.1 Independent reviews

Four reviews on 2026-10-19, blind to each other: protocol and
security, runtime and concurrency, Windows paths, trust of the screens.
No critical or high finding. Every finding and what became of it is in
`docs/DESIGN_QUESTIONS.md` P7-23. The owner deferred how a name is set
apart in a sentence of the client (section 14.2) and accepted that
EndpointUpdate counts against the control rate (2026-10-19).

## 4. Order of the work

A, correctness and security: BL24, BL25, BL2 with BL41 and BL42, BL3,
BL26. B, runtime: BL4 with BL30, BL1, BL13 with BL10, BL39, BL23, BL28,
BL29, BL31, BL32, BL27, BL40. C, interface: BL5, BL38, BL43, BL48, BL44,
BL45, BL46, BL47, BL49, BL50, BL36. D, documents: BL6 to BL12, BL33 to
BL35, BL37, BL51, BL52, BL7.

## 5. Final report

Written on 2026-10-24 and accepted by the owner the same day. Verified
candidate `bbf1279439ea9f58703e10415f2ffa2f5321eeed`; full-verification
ancestor `740b6715c5e1c3c488f3397624d6eb764e763eec`. Part A is the
record of the targeted re-verification of `bbf1279`, part B the report
the brief asks for.

### Part A. Targeted re-verification of `bbf1279`

#### A1. Previous full candidate

`740b671` (2026-10-19, "Record the owner's decisions on two findings of
the reviews"). It received the complete Phase 7 final verification
(section 30 of the brief): builds and tests on Windows (Rust 1.95.0 and
1.85.1) and Linux (1.95.0, 1.85.1 and 1.99.1), deny, audit, the
dependency review, every fuzz target and the long campaigns, the private
Tor network, fail-closed networking, the desktop scenarios and screens,
the packages, and the full mutation baseline of 626 Linux faults plus
the 14 faults of the Windows list. `cargo deny` failed on it, on three
advisories of unmaintained D-Bus crates under the accessibility bridge
(RUSTSEC-2026-0370, 0371, 0372); with the owner's approval they are
ignored with their reasons (`fce59a1`, `d22adda`), production code and
lockfile unchanged, and deny and audit passed on `d22adda` and again on
`ba12472`.

#### A2. New candidate

`bbf1279` (2026-10-22). Ancestry from `740b671`:

| Commit | Change | Kind |
| --- | --- | --- |
| `fce59a1` | ignore RUSTSEC-2026-0370 and 0371 (unmaintained D-Bus crates under the accessibility bridge), with reasons | dependency policy |
| `d22adda` | ignore RUSTSEC-2026-0372 (their derive macros) | dependency policy |
| `e6d5a75` | `tests/runtime.rs`: the MR36 test also checks that the stream of the attempt closes within a quarter of the handshake timeout | test |
| `ba12472` | `mutation/affected.py` and `mutation/README.md`: incremental selection of faults | tooling |
| `801bcbf` | `tests/store.rs`: `EXHAUSTIVE_TEST_TIMEOUT` and `run_exhaustive` for the exhaustive crash tests | test |
| `bbf1279` | `mutation/README.md`, `STATUS.md`, `DESIGN_QUESTIONS.md` 14.2: BL54 recorded and deferred | documentation |

All six commits are by `firebirks <336470935+firebirks@users.noreply.github.com>`
as author and committer, with no trailers.

#### A3. Exact diff `740b671..bbf1279`

```text
 .cargo/audit.toml                     |   7 +
 STATUS.md                             |   2 +
 crates/monolith-core/tests/runtime.rs |  13 +-
 crates/monolith-core/tests/store.rs   |  31 ++++-
 deny.toml                             |  10 ++
 docs/DEPENDENCIES.md                  |  18 ++-
 docs/DESIGN_QUESTIONS.md              |   6 +-
 mutation/README.md                    |  60 +++++++++
 mutation/affected.py                  | 246 ++++++++++++++++++++++++++++++++++
 9 files changed, 386 insertions(+), 7 deletions(-)
```

`ba12472..bbf1279`: `STATUS.md`, `crates/monolith-core/tests/store.rs`,
`docs/DESIGN_QUESTIONS.md`, `mutation/README.md`.

#### A4. What did not change

`git diff --name-only 740b671 bbf1279` over `Cargo.lock`, every
`Cargo.toml`, `rust-toolchain.toml`, `.cargo/config.toml`, every
`build.rs`, every `crates/*/src/`, `mutation/run.py`,
`mutation/faults.py`, `fuzz/fuzz_targets/` and `clippy.toml`: empty.
Production code, Cargo manifests, the lockfile, the toolchain, the
mutation runner and the fault definitions are byte-identical. `Cargo.lock`
has SHA-256 `91fd0c057ba1576db043396fd1f46cd16cc36d9b25d461f692bc104c3b415e78`
on `740b671`, `ba12472` and `bbf1279`; `cargo tree --workspace -e all
--target all` and the resolve of `cargo metadata` hash the same on
`740b671` and `ba12472`, whose manifests and lockfile are those of
`bbf1279`.

The semantic test changes are two: `tests/runtime.rs` (MR36) and
`tests/store.rs` (the bound of the exhaustive crash tests). The
tooling change (`affected.py`) selects faults and runs nothing.

#### A5 to A7. Windows

Worktree at `bbf1279`, clean before and after (0 changed paths). rustc
1.95.0 (59807616e 2026-04-14) and 1.85.1 (4eb161250 2025-03-15).

| Step | Command | Result |
| --- | --- | --- |
| fmt | `cargo fmt --all -- --check` | exit 0 |
| clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 (38 s) |
| clippy, fixtures | the same with the test-fixture features | exit 0 |
| core, 1.95.0 | `cargo test -p monolith-core --locked` | 17 test runs (16 binaries and the doctests), 325 passed, 0 failed, 1 ignored (393 s) |
| core, 1.85.1 | `cargo +1.85.1 test -p monolith-core --locked` | 325 passed, 0 failed, 1 ignored (369 s) |

#### A8 to A12. Linux

Container `monolith-verify:phase7`, source by `git archive` of
`bbf1279`. rustc 1.95.0, 1.85.1, 1.99.1 (58cf8f9c3 2026-10-13).

| Step | Result |
| --- | --- |
| fmt | exit 0 |
| clippy, warnings denied | exit 0 (98 s) |
| clippy, fixtures | exit 0 |
| core, 1.95.0 | 325 passed, 0 failed, 1 ignored (399 s) |
| core, 1.85.1 | 325 passed, 0 failed, 1 ignored (362 s) |
| core, 1.99.1 | 325 passed, 0 failed, 1 ignored (313 s) |

The ignored test in all five runs is
`a_service_published_again_from_its_key_is_reached_again` ("needs the
private Tor network of tests/tor-network"); the private network stage
inherited from `740b671` covers it. No log holds "did not end in time"
or "the exhaustive crash test did not end in time".

#### A13. The store crash tests under load (test B)

The test that failed 12 of 12 on `ba12472`: four copies of the
`store` test binary at once, three rounds, in WSL on 8 CPUs. The binary,
`store-6b86cbde12e69c0c`, was built from `bbf1279` in a target directory
of its own, SHA-256
`1a007fc891c4a991dc93906b7e63faf9a0730666669ad08e7ba5055fa1116de9`.

| Round | Copies | Result |
| --- | --- | --- |
| 1 | 4 | 16 passed, 0 failed each, 224 to 225 s |
| 2 | 4 | 16 passed, 0 failed each, 245 to 246 s |
| 3 | 4 | 16 passed, 0 failed each, 256 to 257 s |

12 of 12 green.

#### A14. Mutation: how the faults were selected

`python3 mutation/affected.py bbf1279` over the results of `740b671`
(phases 1 to 7 and 7w) overlaid with those of `ba12472` (phases 1 to
7); the last file that has a fault gives its evidence. A test file that
changed reruns the faults whose test crates include its crate and that
survived, were caught by a timeout or with no test named, or have
failing tests named in that file.

Selected: 78 of 640. phase1 18, phase2 16, phase3 9, phase4 18,
phase5 5, phase6 3, phase7 9, phase7w 0. Reasons: 68 a failing test of
`tests/store.rs` on record, 5 caught by the timeout, 1 non-zero exit
with no test named (K4), 4 survived (CAP2, CAP3, S15, S24). MR36 is not
selected: its evidence is from `ba12472`, and nothing it depends on
changed since. B3, B4, Q14, Q18 and Q29 are selected because their
`ba12472` record names `*_is_atomic` tests.

The incremental selector reduced the rerun set, but the validity of
this targeted re-verification does not rely solely on the new selector:
`740b671` had already completed a full 640-fault campaign, and the only
semantic test changes between candidates are `tests/runtime.rs` and
`tests/store.rs`.

phase7w: the Windows results of `740b671` were placed in the Linux
clone before the selection, so the selector saw all eight result files
and selected 0 of the 14 `phase7w` faults. (On `ba12472` the Linux
selector had reported the Windows-only list as lacking results because
its result file was not present in that environment. Re-running the
selection with all eight result files, including the Windows Phase 7
results, selected 0 of 14 `phase7w` faults.)

#### A15. Mutation: what ran

WSL Debian, a clone at `bbf1279` (0 changed files), 4 workers,
work trees made fresh. Each phase measured its own unmodified run:

| Phase | Faults | Unmodified run | Timeout | Outcome |
| --- | --- | --- | --- | --- |
| phase7 | 9 | 507 s | 4056 s | 9 caught |
| phase6 | 3 | 456 s | 3648 s | 3 caught |
| phase5 | 5 | 452 s | 3616 s | 5 caught |
| phase4 | 18 | 451 s | 3608 s | 18 caught |
| phase3 | 9 | 441 s | 3528 s | 6 caught, 3 survived |
| phase2 | 16 | 404 s | 3232 s | 12 caught, 4 survived |
| phase1 | 18 | 409 s | 3272 s | 16 caught, 2 survived |

78 of 78 faults have exactly one result each; every verdict is caught or
survived (no invalid). Every verdict equals the `740b671` verdict of the
same fault. For the 11 phase4 faults whose `740b671` outcome exists only
in the log of the interrupted run (CS1, CS2, CS3, CS4, CS6, CS11, CS13,
CS14, CS23, CS35, CS36; see A18), the log says caught, as now.

#### A16. Survivors

B3, B4, CAP2, CAP3, Q14, Q18, Q29, S15, S24: the nine expected survivors
of the baseline, exactly. B3, B4, Q14, Q18 and Q29, counted caught on
`ba12472` only through `*_is_atomic` tests, now survive with no failing
test: they stay historical survivors, not new catches.

#### A17. Faults caught by the timeout, and K4

| Fault | `bbf1279` | `740b671` |
| --- | --- | --- |
| Q1 | timeout, plus four named tests of `monolith-tor` (`a_failure_of_tor_ends_the_proof` and three more) | the same tests, plus four `*_is_atomic` tests |
| CS30 | timeout, plus three named tests of the supervisor and the front end | the same tests, plus five `*_is_atomic` tests |
| MR9 | timeout, plus four named tests of the scheduler | the same |
| MR25 | timeout only | timeout only |
| MR26 | timeout only | timeout only |
| K4 | non-zero exit, no failing test named | the same |

The classification is kept: each is caught as before, by the same
tests, and no new test catches one before the timeout. The `*_is_atomic`
names in the `740b671` lists of Q1 and CS30 were the harness-timeout
noise of A19; they are gone.

#### A18. Phase 4 faults without JSON on `740b671`

On 2026-10-20 WSL stopped at 05:22 during phase4 of the full run of
`740b671`, with 37 of 123 faults done; the other 86 were run again from
scratch. The 37 have their outcome in the log of the interrupted run and
no JSON. None relied on a mapping from Phase 6: all 37 were run again on
`ba12472` (JSON), and 11 of them again on `bbf1279`. The selector saw
the `ba12472` JSON for all 37.

#### A19. The failures of the store crash tests

Found on `ba12472`: under four mutation workers the `*_is_atomic` tests
of `tests/store.rs` failed in runs whose fault had nothing to do with
the store, so B3, B4, Q14, Q18 and Q29 looked caught. Every catch was
then held to one rule: a fault is caught only when a test that bears on
it failed.

Diagnosis, no production code or timeout changed until it was done:

- alone, every crash test passes: the invitation test in 53 to 70 s,
  the others in 9 to 34 s;
- four copies of the store binary at once: 12 of 12 failed; four copies
  of all 16 test binaries of `monolith-core`: 4 of 4 failed; all 110
  panics were "the test did not end in time" at
  `tests/common/mod.rs:40` (the 120 s bound of `common::run`), no
  assertion failed;
- four copies with that bound at 3600 s: all passed in 207 to 209 s,
  CPU time rising in every sample, threads mostly running, no stall,
  available memory never below 10165 MiB and no swap;
- one core, `--test-threads=16`, binaries in separate target
  directories checked by SHA-256: the 120 s binary failed twice with 8
  timeouts and nothing else; the 3600 s binary passed twice, in 316 and
  339 s.

Cause: a harness bound under CPU contention. `crash_everywhere` opens the
vault, with its key derived at `KdfParams::FLOOR`, twice for every step
of a write and every crash outcome; under four workers on eight CPUs
each of these tests had about a fifth of a core and took 100 to 170 s.
Not a liveness defect of production.

Fix (`801bcbf`, test only): `run_exhaustive`, with
`EXHAUSTIVE_TEST_TIMEOUT` of 600 s and its own message ("the exhaustive
crash test did not end in time"), for `crash_everywhere` and
`an_announced_successor_is_atomic`; every other test keeps the 120 s of
`common::run`. The bound is a guard well above the measured worst case,
not a promise that it holds on any machine.

Effect on `740b671`: 55 of its 603 JSON results named `*_is_atomic`
tests, and none was caught by them alone. Its catches stand. For the
faults that do not reach the store, those failures were harness-timeout
noise; for faults that break what the store tests exercise (T4, T6, K2,
K3, C7, R11, H2, H4, CR4, CR9, CS45, CS64 among them) they may also be
genuine.

On `bbf1279`: 20 of the 78 results name tests of `tests/store.rs`
(T4, T6, K2, K3, C7, R11, H2, H4, CR4, CR9, CS6, CS11, CS13, CS14,
CS23, CS35, CS36, CS41, CS45, CS64). Every one of them also has failing
tests outside `tests/store.rs` that bear on the fault, and no fault is
caught by `tests/store.rs` alone. Those faults break what the store
tests exercise (the transport keys, the KDF, cards, credentials, the
vault and its writes). 14 of the 20 runs ended within 600 s in all, so
no crash test in them can have reached its new bound; the runner keeps
no panic messages to say more. The one store test of CS41,
`a_restart_on_disk_reproduces_the_durable_state`, is not a crash test
and keeps the 120 s bound.

The runner still counts any failing test (BL54, deferred with the
owner's approval on 2026-10-22); until it tells the failures apart, the
outcomes are classified by hand (`mutation/README.md`).

#### A20. Procedure incidents

- `ba12472`: automatic launcher failed before mutation execution -> no
  fault had started -> machine returned idle -> same selected set
  launched manually. The launcher is scratch tooling outside the
  candidate.
- `bbf1279`: the owner had the run paused on 2026-10-22 after phase7
  ended (14:24:45, its JSON written) and before phase6 produced any
  output; WSL was shut down. The run resumed on 2026-10-23 at 22:13 with
  the remaining phases, fresh work trees, the same candidate and
  selection. phase7 was not run again: its result file was complete.
- A watchdog then watched the host: below 4 GB available it dropped the
  WSL page cache (19 times between 22:28 and 05:51; available memory
  never fell below 2.39 GB); below 2 GB it would have stopped the run
  and rerun every fault in flight during the stop. It never stopped it:
  no fault ran during a pause, and the list of faults to rerun was
  empty. The host restarted at 09:22 on 2026-10-24, after
  the run had ended at 06:10.

#### A21. Stages inherited from `740b671`

Production code, Cargo manifests and lockfile are byte-identical; only
two regression tests, the mutation tooling and documents changed, plus
the dependency policy, whose checks were rerun. These results of
`740b671` remain applicable:

- the full workspace tests on Windows (1.95.0, 1.85.1) and Linux (1.95.0,
  1.85.1, 1.99.1) outside `monolith-core`, which no change touches;
- the `cargo doc` builds and the 1.85.1 checks;
- the dependency duplicate review (`cargo tree -d`): the graph is
  identical;
- every fuzz target and the long campaigns: the targets and the code
  they reach are unchanged;
- the private Tor network, fail-closed networking, the restart, retry
  and rotation scenarios: production code unchanged;
- the Windows locality tests and the Windows mutation list (14 of 14):
  the selector, given all eight result files, selected none;
- the desktop scenarios, screens, measurements and the idle client;
- the packages and their checksums;
- the mutation results of the 562 faults not selected: those of
  `740b671`, or of `ba12472` for the faults run again there (MR36,
  which survived on `740b671`, among them);
- deny and audit on `d22adda`, with the policy exception.

Rerun because their evidence could change: fmt and clippy, the
`monolith-core` tests on five combinations, test B, the 78 selected
faults; and on `ba12472`, cargo deny and cargo audit, because the
policy changed (cargo-deny 0.20.2: advisories, bans, licenses, sources
ok; cargo-audit 0.22.2: 445 crates, exit 0). `deny.toml` and
`.cargo/audit.toml` are the same on `ba12472` and `bbf1279`.

#### A22. Tooling lessons

- Two source variants built into one `CARGO_TARGET_DIR` overwrite each
  other's test binary (cargo hashes a member by its path in the
  workspace): one target directory per variant, and the SHA-256 of the
  binary that ran on record. A first one-core trial was invalid for this
  reason and was repeated.
- libtest takes its thread count from the CPU affinity: under `taskset`
  to one core, tests run one at a time unless `--test-threads` is given.
- A fault counts as caught by any failing test, also one that fails for
  another reason under load (BL54); outcomes need the hand rule until
  the runner tells them apart.
- `run.py` writes its results only when a phase ends, and a stopped run
  keeps its clocks going, so a pause has to be followed by running again
  every fault that was in flight.

#### A23. Verdict

`740b671` received the complete Phase 7 final verification, in which
deny passed only with the owner's post-freeze dependency-policy
exception (`fce59a1`, `d22adda`; production candidate unchanged).
`bbf1279` differs only in two regression tests
(`crates/monolith-core/tests/runtime.rs`,
`crates/monolith-core/tests/store.rs`), the mutation selector and its
documentation, the dependency policy and documents; production code,
Cargo manifests and lockfile are byte-identical. The affected
verification stages were rerun on `bbf1279` (deny and audit on
`ba12472`, whose policy files are those of `bbf1279`). All unaffected
results from `740b671` remain applicable. `bbf1279` is the Phase 7
verified candidate.

TARGETED RE-VERIFICATION PASSED

### Part B. Phase 7 final report

#### 1. Starting baseline

`main` at `20a6779` (2026-10-18): `phase-6-complete` (`92de84a`, whose
production code is that of `68ac655`, the commit Phase 6 was verified
on) and two documentation commits. The production code of Phase 5 was
verified on `91bf22c`.

#### 2. Verified commit

`bbf1279439ea9f58703e10415f2ffa2f5321eeed`, by targeted re-verification
(Part A) on top of the full final verification of
`740b6715c5e1c3c488f3397624d6eb764e763eec`. Production code is that of
`740b671`.

#### 3. Branch and history

`phase-7-security-hardening`, 33 commits from `20a6779` to `bbf1279`
(27 to `740b671`), 99 files changed, 7622 insertions, 742 deletions.
All 33 commits have `firebirks <336470935+firebirks@users.noreply.github.com>`
as author and committer and no trailers; no merge commits; nothing
rewritten. The commits after `bbf1279` change documents only: the
owner's reading of the bound on unbroken runs (BL47), this report, the
record of the completion, and the removal of `STATUS.md`, which holds
the handover, when the phase is closed.

#### 4. Backlog inventory

BL1 to BL54 (`STATUS.md` section 2), every item accounted for:

| Outcome | Items |
| --- | --- |
| fixed in code, each with a regression test that failed before | BL1, BL2, BL3, BL4, BL5, BL13, BL24, BL25, BL26, BL27, BL28, BL29, BL30, BL31, BL32, BL36, BL38, BL39, BL40, BL41, BL42, BL43, BL44, BL45, BL46, BL48, BL49, BL50 |
| fixed in the tests only: CS29 now fails the tests within about a minute | BL23 |
| small corrections | BL6, BL7, BL35, BL52 |
| documented | BL8, BL9, BL10, BL11, BL12, BL33, BL34, BL37, BL51 |
| shown not to overflow, a reading of the limit the owner confirmed (2026-10-24) | BL47 |
| measured and accepted | BL19 |
| deferred with the owner's approval | BL14 to BL18, BL20 to BL22 (2026-10-18), BL53 (no stable compiler fixes rust-lang/rust#164250), BL54 (2026-10-22) |

All four medium items of 13.3 and the three of 13.4 (BL1, BL2, BL3,
BL13, BL24, BL25, BL26) are fixed.

#### 5. Key-rotation fallback (BL24, P7-1)

An attempt dials the cards of `link::dial_plan` in order: the authorized
successor, then the active key, the latter only when the successor did
not answer (its handshake ended before message 2 authenticated the
responder), as `PROTOCOL.md` 4 and 11.4 require. A successor that
answered, or a failure of Tor, decides the attempt. No further card is
dialed once the attempt is over for its caller, and `link::dial` checks
the plan again before opening a stream; the attempt is reported once.
Faults HR1 to HR4; the private network stage covers a rotation across
restarts.

#### 6. UI trust and names (BL25, P7-2, P7-23)

A name a sender chose is never inside a sentence of the client: the
request card and the Accept dialog show it, and the sender's message, in
a box of their words under labels of the client. A name in a sentence of
the client is a span of its own, drawn in the strong style, never in
quotation marks the client adds; runs of spaces are drawn as one. The
avatar of a request has no accessible name, and a row reads its preview
last. Accept suggests the name as sent. Faults HR5 to HR8 and the
review faults.

#### 7. Windows path and file locality (BL2, BL41, BL42, P7-3)

`monolith-desktop::local` checks on Windows every path the user gives (a
card file, the folder a card is saved in, the cookie file, `--data-dir`,
`--config`, the default places): by name (absolute on a drive, no device
name, no colon, no trailing dot or space), the root resolved first, then
each part (no link or junction, nothing offline or cloud only), then
where it resolves (a drive, not a share). A card file is opened without
following a reparse point or recall and read only if it is the regular
file looked at, on the disk. Limits are documented (`DESKTOP.md` 10): no
file ID without unsafe code, so the identity is size and times; a
junction placed by a process of the user between check and open is not
defended against.

#### 8. Tor cookie handling (BL39, BL42, P7-11)

The cookie path is checked as a local file (section 7) at the start. On
Windows the name Tor reports is compared with the configured one without
the case of ASCII letters; any other difference is refused, so a
mismatch fails closed as before, and only the configured file is ever
read. Finding the cookie by the name Tor gives is deferred (BL20).
Faults HR45, HR46 (Windows).

#### 9. File-protocol handling (BL3, P7-4, P7-23)

Without file transfer: each session answers a FileOffer at once with
FileReject and remembers its identifier among the last
`MAX_ENDED_TRANSFER_IDS`; a FileAbort for it, or the same offer again,
is discarded; any other file message (a chunk, completion, accept or
reject for that offer, or anything for an unknown identifier) ends the
session as a violation. A later session treats a replayed message as a
violation. A Profile is accepted only from an accepted contact on a
confirmed session, its texts are checked, and it is dropped. Faults HR17
to HR20.

#### 10. Message-store compatibility (BL26, P7-5)

Each stored format has ceilings, equal to the limits it was made with;
reading and writing use them, what is added uses the limits of the
build, which a compile-time assertion keeps at or below them. A store
above a lowered limit opens, is kept and grows no further. The vault
likewise. Raising a limit above a ceiling needs a new format version.
Faults HR21 to HR23; MR12, MR13, MR14, MR45 retargeted to the same
defects.

#### 11. Session activity (BL1, P7-6)

Activity is refreshed only by a ChatMessage sent, one taken in and
stored, and a MessageAck that delivered a message, so acknowledgements
of nothing or messages sent again no longer keep an idle session open.
Faults HR24, HR25.

#### 12. Duplicate-session probe (BL4, BL30, P7-6)

The probe and the Ping spend their timeouts only while the task reads.
One Ping is outstanding at most; a late Pong is taken and dropped; a
Ping not answered within `PONG_TIMEOUT` ends the session with Close.
When a probe ends the credentials are looked at again, and a session
whose key was retired meanwhile loses. Faults HR26 to HR30.

#### 13. Unicode presentation (BL5, BL47, BL49, P7-14)

Tag characters and every character a display name rejects as invisible
or direction-changing are drawn as marks, plus the Egyptian format
controls. A run without a break opportunity breaks at the width of its
bubble, so nothing overflows (BL47; nothing is inserted into the text,
which would also be copied). The owner confirmed this reading on
2026-10-24: `MAX_RENDERED_UNBROKEN_RUN` bounds the rendering and
rewrites no text, and it holds when the run stays inside the bubble.
Name fields take at most 128 bytes and 64
characters. Faults HR53, HR55; fuzz target `ui_present`.

#### 14. Tor diagnostics (BL38, P7-15)

With the SOCKS proof on, a status query whose control endpoint failed
reports the SOCKS endpoint `Unproven` (not tried), and "Check Tor" says
`NoControl` (the control port cannot be reached) instead of "Tor is not
running". Nothing is connected to the SOCKS endpoint without the proof,
as before. Faults HR56, HR57; UI42 retargeted.

#### 15. Retry-event correctness (BL27, BL28, BL29, P7-7, P7-12)

`DeliveryDeferred` is reported only when the scheduler goes on (record
current, identity open, no store failed, messages queued, no session),
also after a failed dial. Block and delete name the record inside their
work; a send looks at its record inside the change that queues the
message. `SendError::ShuttingDown` and `SendError::Randomness` say what
happened. Faults HR31 to HR34, HR49 to HR52; MR36 caught by its test.

#### 16. Window and zoom persistence (BL43, BL44, P7-16)

The saved size is multiplied by the zoom, so the window keeps its size
on the screen. The window thread no longer looks at the disk: the
platform markers are found at the start, the downloads folder is chosen
on the files thread. Faults HR58, HR59.

#### 17. Avatar contrast (BL48, P7-14)

Five light avatar colours darkened with their hue kept; white initials
are at least 4.5 to 1 on every colour of both themes, and the contrast
test covers them. Fault HR54.

#### 18. Rate limits (BL13, P7-8)

Five of the seven rates of `RESOURCE_LIMITS.md` 6 are enforced: the four
of what a contact sends and `CONTACT_SESSION_RATE`, as token buckets
keyed by what a session authenticated, never shared between contacts,
at most `MAX_CONTACTS` buckets that are not full held per kind and
identity. Above a message rate
the session waits for its token and reads nothing; above the session
rate a session is closed once admitted. `INBOUND_CONNECTION_RATE` and
`UNKNOWN_SESSION_RATE` are not enforced, with the reason in P7-8: under
Tor one bucket for all peers would let one peer shut out every contact
and every invitation; the handshake budget, the stranger budget and the
request quotas bound what they would. EndpointUpdate counts against the
control rate (accepted by the owner, 2026-10-19). Faults HR35 to HR43.

#### 19. Resource amplification (BL10, BL19, BL37, BL40, P7-9, P7-10, P7-21)

Every stored message rewrites the store: bounded by the rate, after a
burst, to at most 5 writes a second per contact (about 80 MiB/s with a
full store) and
about 116 000 writes until the conversation is full; documented, a
store writing only what changed is left for later. The memory of a write
was measured: 64 to 100 MiB in all. The SOCKS proof costs about 1.8 ms
a failed dial, accepted. A rename (and a removal) another process blocks
on Windows is retried for about 1.6 s on sharing or access errors only
(HR44, HR75).

#### 20. Documentation corrections (P7-20 and the rest)

ADR 0002, ADR 0008, ADR 0009, `ARCHITECTURE.md` 1.1, `PROTOCOL.md` 6.2,
8.2, 8.7, 13 and 14, `RESOURCE_LIMITS.md`, `SECURITY_INVARIANTS.md` (S13,
S15), `STORAGE.md` 3.4, 3.5, 3.6 and 3.8, `CRYPTOGRAPHY.md` 2 and 8,
`THREAT_MODEL.md`, `DEPENDENCIES.md`, `DESKTOP.md` 3, 5, 6, 8 and 10,
`PLATFORM_TAILS.md` 5, `TEST_PLAN.md`, `DESIGN_QUESTIONS.md` 12.3, 12.5,
12.6 and section 14, `mutation/README.md`. The broken intra-doc links are fixed and the CI
builds the documentation with warnings denied.

#### 21. Tests

On `740b671`: 76 test runs (64 binaries and 12 of doctests); Windows 1264 passed, 0 failed, 2
ignored; Linux 1252 passed, 0 failed, 2 ignored (the Tor-network test,
covered by the private network stage, and a measurement run by hand).
On `bbf1279`: `monolith-core` 325 passed, 0 failed, 1 ignored on five
combinations; test B 12 of 12. Phase 7 added regression tests for every
fix, property tests against independent models (rates, file messages,
lowered limits, notices of other identities; P7-22), and Windows-only
tests of locality, renames and the cookie name.

#### 22. Windows and Linux matrix (`740b671`)

| Stage | Windows | Linux |
| --- | --- | --- |
| fmt, clippy, clippy with fixtures, doc | exit 0 | exit 0 |
| workspace tests | 1264/0/2 | 1252/0/2 |
| package | zip, 4 575 549 bytes, SHA-256 `534b6ae2dc5f57204d3114842a9ee63ee722c72179bfd738fc2130bb71015658` | tar.gz, SHA-256 `964672c3e57ddd06663a65b7b208a472559a52368ae91793ee681bfe46d0da1e`, notices of 247 crates |
| desktop scenarios | passed (32 s) | passed (38 s) |
| screens | passed | passed |
| measurements, scrolling 2000 messages of 1.5 KB | 0.21 ms median | 0.15 ms median |
| idle client | 150 MB working set, 16 ms CPU in 10 s | - |
| Tor smoke | passed | - |

#### 23. Toolchain matrix

| Toolchain | `740b671` | `bbf1279` |
| --- | --- | --- |
| 1.95.0 | Windows and Linux tests | `monolith-core`, Windows and Linux |
| 1.85.1 (MSRV) | Windows and Linux tests and checks | `monolith-core`, Windows and Linux |
| 1.99.1 (stable of the day, 2026-10-13) | Linux tests | `monolith-core`, Linux |
| nightly 1.101.0 (2026-10-10) | fuzzing | - |

BL53: rust-lang/rust#164250 was still open, with no fix in any stable
compiler, 1.99.1 included, when it was checked on 2026-10-19; the
deferral stands.

#### 24. Fuzzing (`740b671`, cargo-fuzz 0.13.2)

20 targets, 26 s each, no crash: base32, contact_card,
contact_card_text, message_body, frame_plaintext, frame_stream,
text_fields, session_sequence, credential_sequence, handshake_responder,
handshake_initiator, session_frames, tor_control_reply, socks_reply,
vault_payload, contact_store, message_store, message_sequence,
ui_present, ui_model_sequence. Long campaigns on the surfaces Phase 7
changed, 601 s each, no crash: ui_present (1.1 M runs), message_store
(8.4 M), message_sequence (93 k), session_sequence (7.5 M),
ui_model_sequence (2.0 M). No new target (P7-22): `ui_present` covers
the larger set of marks, `message_store` holds the decoder to the
ceilings.

#### 25. Mutation results

`740b671`, full: 626 Linux faults, each with exactly one outcome; caught
except the nine expected survivors and MR36; MR25 and MR26 caught only
by the runner's timeout, MR9, CS30 and Q1 by the timeout and named
tests. Windows list: 14 of 14 caught (HR12 to HR16,
HR44, HR46, HR69 to HR75). Phase 7 list: 71 Linux faults (HR1 to HR85
without the Windows ones), all caught.

MR36 survived on `740b671`: since P7-7 a failed dial for a record that
is gone reports nothing, and the test watched only events. The defect
stays real (the stream stays open until the handshake times out); the
test now checks that the stream closes within a quarter of the timeout
(`e6d5a75`). On `ba12472` MR36 was caught in 545 s by
`blocking_or_deleting_a_contact_ends_the_attempt_in_progress`.

`bbf1279`, targeted: 78 of 78, verdicts equal to `740b671`, survivors
the nine expected, no catch by `tests/store.rs` alone (Part A). The
runner's safeguards against stale binaries are unchanged (`run.py`
byte-identical).

#### 26. Phase 1 to 6 non-regression

Every test of Phases 1 to 6 is green on `740b671`; `monolith-core` again
on `bbf1279`. Every earlier mutation list is present and unweakened (B,
S, CAP, Q, CS1 to CS123, MR1 to MR57, UI1 to UI160 and the rest); the
patterns that moved were retargeted to the same defects (MR12, MR13,
MR14, MR45, MR48, MR49, MR54, Q27, UI23, UI42, UI130). The expected survivors are exactly
the nine of the baseline. No security invariant was weakened.

#### 27. Independent reviews

Four reviews on 2026-10-19, blind to each other (protocol and security,
runtime and concurrency, Windows paths, trust of the screens), over
`20a6779..f1f0d78`: no critical or high finding. Each finding was
reproduced first where it could be; what was fixed has a test that
failed before; the rest is said in the documents, accepted as a cost
(P7-23), or was not reproduced. The owner deferred a name set apart only
by weight (14.2) and accepted that EndpointUpdate counts against the
control rate. Faults HR68 to HR85.

#### 28. Tor E2E (`740b671`, Tor 0.4.9.13, Chutney)

The private network run passed every step: a dial and one message each
way; publication again from the key in memory; two nodes over vaults;
B's restart keeping contact, identity and service; A's key rotation held
across B's restart, then promoted, the old session withdrawn and the
retired key kept; a block during an open session; a second identity with
invitation mode, a capability and deletion; Tor B's restarts with the
services published again; no traffic without SOCKS or Tor; queued
delivery while B is away, delivered once; a crash after storing and
before acknowledging, the retry not stored again; A's crash with a
message queued; Tor A's restart with a message queued; a block with a
message queued; a second identity sending both ways at once; two desktop
clients driven through their window code; the control connection lost
reported. Every node run contacted only loopback and Unix sockets, no
DNS. Fail-closed networking (T-NET-1): passed, including a network
namespace with loopback only.

#### 29. Windows locality verification

The Windows tests of `monolith-desktop` (junctions, symbolic links,
offline and cloud-only files, device names, streams and trailing dots,
a card swapped for one of the same length, the downloads junction, the
save folder), of `monolith-storage` (blocked renames and removals) and
of `monolith-tor` (the cookie name) passed on 1.95.0 and 1.85.1. The
Windows mutation list caught 14 of 14. Not covered, as documented: a
mapped drive is found when its root resolves, after Windows looked it
up; drives backed elsewhere and synced folders pass.

#### 30. Remaining limitations

- Nothing has been audited; the software is experimental.
- Windows locality: no file ID and no open relative to a held folder
  without unsafe code; the race of a junction placed by a process of the
  user is not defended against.
- `INBOUND_CONNECTION_RATE` and `UNKNOWN_SESSION_RATE` are not enforced
  (P7-8).
- Each stored message rewrites the store; bounded, not removed.
- Decoded message texts are not erased from memory (`CRYPTOGRAPHY.md` 8).
- Whether a scanner holds the store files on Windows was not
  established; what a control connection costs Tor was not measured.
- Accepted in P7-23: marks on some joiners and soft hyphens inside
  words, unmarked variation selectors, a blank bubble for U+2800, the
  pill of a long card name on a narrow window.
- MR25 and MR26 are caught only by the runner's timeout; MR9, CS30 and
  Q1 by the timeout and named tests.
- The mutation runner counts any failing test (BL54).

#### 31. Deferred work

With the owner's approval: tests with screen readers, shaping and bidi,
a message list reachable by keyboard, keeping unread counts and
declines (BL14 to BL17); blocking a key without a record (BL18); finding
the Tor cookie by the name Tor gives (BL20); installers and signing
(BL21); the client tried by hand on Tails and Whonix (BL22); building
with a stable compiler that fixes rust-lang/rust#164250 (BL53); a
mutation runner that tells the failures apart (BL54); setting a name
apart in a sentence of the client by more than weight (P7-23). Out of
the phase: file transfer, the Double Ratchet, groups, mobile, voice and
video, read receipts, typing indicators, presence, push, cloud sync, a
new transport or identity model.

#### 32. Final verdict

Every item of 13.3 and 13.4 is accounted for; every medium finding is
fixed; the dial plan falls back as the protocol says; sender names
cannot read as the client's words; the Windows local-file claims match
the checks; unsupported file messages are handled as the protocol says;
the store's limits are migration-safe and documented; invalid traffic
no longer extends a session; the selected rates are enforced; the Tor
diagnostics claim only what was observed; the Phase 6 UI defects are
fixed; documentation matches production; every regression of Phases 1
to 6 is green; the mutation baselines are protected and every Phase 7
fault is caught; the reviews left nothing critical or high; and the
final verification passed on one frozen production state, `740b671`
(deny with the owner's post-freeze dependency-policy exception, production
candidate unchanged), with the targeted re-verification of `bbf1279`
passed.

PHASE 7 COMPLETE
