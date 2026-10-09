# Phase 4 final report (DRAFT)

Status of this document: DRAFT. Not committed. It becomes `STATUS.md`
section 5.3 only after every stage of the final verification has
completed. Phase 4 is OPEN / NOT COMPLETE until then.

Placeholders for results that are not known yet are written as
`[PENDING: ...]`. Nothing in them is inferred.

Sources: the repository at `970316f`; the outputs of the final
verification kept on the branch `verification-970316f`
(`verification/970316f/`); `git` on the local clone.

---

## 1. Verified candidate

- Candidate commit: `970316fc76d0ea19c1c65239b16556253a2894c8`
  ("Record the commits before the freeze and their checks").
- Branch: `phase-4-contact-store`.
- Phase 3 baseline: `2f27dcb99df0f86bb1c539ff69b2323a43d75a05`.
- Every result of the final verification below was produced on this
  exact commit, with a clean tree: stages 1 and 2 in the repository
  checkout at `970316f`, stage 3 in copies made by `mutation/run.py` from
  `git archive` of `970316f` (each copy carries a `.mutation-commit`
  marker with that hash). No file changed between the stages.

## 2. Git ancestry and history

- `2f27dcb` is an ancestor of `970316f` (`git merge-base --is-ancestor`).
- 171 commits in `2f27dcb..970316f`.
- Merge commits in that range: 0.
- No history rewrite: the remote branch was updated four times, by push,
  each a fast-forward of the previous one (`03a626c` on 2026-10-05,
  `c8322e9` and `289038f` on 2026-10-06, `970316f` on 2026-10-07; each
  checked with `git merge-base --is-ancestor`). No force push.

## 3. Git identity

- `git log 2f27dcb..970316f --format='%an <%ae> | %cn <%ce>'`: 171 of
  171 commits have
  `firebirks <336470935+firebirks@users.noreply.github.com>` as author
  and as committer. No other identity.
- Trailers: none in the range (`%(trailers)` is empty for every commit).
  No `Signed-off-by`, `Co-authored-by` or similar line.

## 4. Repository state

At the time of writing (2026-10-09):

- Working tree of `phase-4-contact-store`: clean.
- `phase-4-contact-store` is level with `origin/phase-4-contact-store`
  (0 ahead, 0 behind), both at `970316f`.
- Pushed:
  - `phase-4-contact-store`, on the owner's request, last on 2026-10-07
    (`970316f`).
  - `verification-970316f`, on 2026-10-08, on the owner's request, to
    continue the mutation run on another machine. It holds one commit
    (`d8f549d`) on top of `970316f` that adds only
    `verification/970316f/` (the logs and results of the final
    verification and the script that runs the remaining mutation
    phases). It is a work branch, not a candidate, and is not to be
    merged.
- Merges: none. `main` is at `6eed860`, unchanged; it holds four
  document commits after `2f27dcb` (`86adac1` to `6eed860`) that are not
  on this branch.
- Tags: none created in Phase 4. The only tag is `phase-3-complete`
  (annotated, 2026-10-03, pointing at `6eed860` on `main`).

## 5. Phase 4 implementation summary

Phase 4 adds one durable, transactional and authoritative contact
subsystem, without any change of Noise XK or of the wire format (the
known-answer vectors of `docs/PROTOCOL.md` 16.1 are unchanged):

- Authoritative durable contact store: one `ContactStore` per local
  identity decides about every card, admits every session and holds the
  credentials of every contact (`crates/monolith-core/src/contacts.rs`).
- Atomic persistence: the encrypted vault (`crates/monolith-storage`),
  replaced atomically, written by a write job that does not depend on
  its caller (`persist.rs`); commit before use (P4-4).
- Unified credential/card evaluation: one classifier
  (`Credentials::relation`) and one table (`evaluate`,
  `contact::decide`) for every source of a card (P4-1).
- Per-contact serialization: one lock per contact under one map lock,
  nothing awaited under a lock (P4-3).
- Session reconciliation and withdrawal: one rule that withdraws every
  session that no longer stands for its contact, after every change
  (P4-5); message 3 only in the admission of the responder (P4-2).
- Invitation handling: durable capabilities and revocations, a request
  queue with handles (P4-12).
- Multi-identity isolation: per-identity stores, random instance numbers
  in every handle id, one vault per installation (P4-6, MI-2, MI-3).
- Stranger budgets: per-identity and per-process limits; eviction of
  the oldest silent stranger (P4-7, P4-8).
- Tor publication recovery: a supervisor that publishes each identity's
  Onion Service again after Tor lost it, with backoff (P4-9).
- Rotation lifecycle and instance binding: durable rotation steps bound
  to a `RotationId`, announcements bound to rotation, contact instance
  and session (P4-10, P4-11, CR-1).

Decisions: `docs/DESIGN_QUESTIONS.md` section 11.2 (P4-1 to P4-15).

## 6. Credential state-machine result

Source: `crates/monolith-protocol/src/credential.rs` (doc at 12-35),
`contact.rs`, `session.rs`; `docs/PROTOCOL.md` 11.4.

- Relations of a card to the credentials (`CardRelation`): `Same`,
  `OlderActiveKey`, `NewerActiveKey`, `Successor`, `NewKey`, `Conflict`,
  `Stale`. Outcomes (`CredentialChange`): `Unchanged`, `Superseded`,
  `Advanced`, `Authorized`, `Pending`, `Promoted`, `Conflict`, `Stale`,
  `NoContinuity`.
- Active: exactly one active card; only the active transport key
  authorizes a contact session (`authorizes`).
- Authorized successor: only from an announcement (EndpointUpdate) on a
  session made with the active key; an announcement on any other session
  gives `NoContinuity`. A successor proven by a handshake is `Promoted`.
- Pending successor: a newer key proven by a handshake, or imported for
  an accepted contact. It is replaced only by a confirmation of the same
  statement; a presented card never displaces an imported one.
- Stale / superseded: a card of the retired key is `Stale`; an older
  active card is `Superseded` from every source and changes nothing.
- Retired: promotion retires the previous key and clears the authorized
  successor; sessions of the retired key are withdrawn.
- Conflict: a card at the same epoch as the active card or the
  successor that states something else; it changes nothing.
- Blocked / deleted: record kinds, not credential states. `decide` gives
  them the record's standing without evaluating the card; an import of a
  blocked identity is refused.
- Epoch monotonicity: the active epoch never goes down; checked by a
  proptest, by the exhaustive table test and by the fuzz target
  `credential_sequence`. There is no separate runtime check; it follows
  from the table.
- No automatic privilege for an unconfirmed newer key: a pending key has
  the standing `PendingSuccessor`, which is not a contact standing; it
  gets no message 3, no contact session and is never dialed. Only
  `Requested` and `Accepted` standings may learn the local identity.
- Tests: `credential::tests::table` against a model
  (`every_card_from_every_source_in_every_state`,
  `the_same_card_has_the_same_relation_whatever_its_source`);
  `contact::tests::every_kind_of_record_in_every_context`,
  `import_and_admission_agree_on_every_card`.

## 7. Unified credential evaluation

Paths that evaluate a card through `contact::decide`:

| Path | Entry | Context |
| --- | --- | --- |
| Inbound admission | `ContactStore::admit_inbound`, `InboundPeer::admit` | `Inbound` |
| Outbound admission | `ContactStore::admit_outbound`, `OutboundPeer::admit` | `Inbound` (same `Source::Proven` as `Outbound`) |
| Import | `ContactStore::import` | `Import` |
| Confirmation of a pending card | `ContactStore::confirm_pending` | `Confirmation` |
| Announcement (EndpointUpdate) | `ContactStore::announce`, after `Entry::stands_for` | `Announcement(session)` |

Paths that use the same credential machinery without `decide`, by
design of what they do:

- `confirm_dial` (the user confirms where to connect): requires
  `Credentials::relation(card) == Same`; it never changes which key
  stands.
- `accept_request`: makes a new contact only for an identity with no
  contact record (none or declined); an existing contact is left as it
  is; a blocked identity is refused.
- Session standing after admission: `Entry::stands_for` and
  `Record::stands`, both through `Credentials::authorizes`; used by
  `mark_accepted`, `announce`, `mark_on`, `session_stands` and
  `reconcile`.
- Dialing: `dialplan::plan` uses `authorized_successor`, `active` and
  `authorizes`; `link::dial` refuses a card the plan does not name.
- Vault load: `Credentials::restore`.

`contact::Context::Outbound` exists but no production path uses it; it
maps to the same source and standing as `Inbound`.

Agreement is tested by `import_and_admission_agree_on_every_card` and
the source-independence table test. The reviews of section 25 found no
path where two of these disagree; this report does not claim more than
those tests and reviews show.

## 8. Persistence and atomicity

Sources: `crates/monolith-storage/src/{vault.rs,dir.rs}`,
`crates/monolith-core/src/persist.rs`, `docs/STORAGE.md` 3,
`docs/SECURITY_INVARIANTS.md` S31.

- Temporary file: fixed name `vault.new` in the vault directory, opened
  with `O_CREAT | O_EXCL | O_NOFOLLOW`, mode 0600; a leftover one is
  removed first.
- Write: `write_all` of the sealed next generation.
- File fsync: `sync_all` before the rename.
- Atomic replacement: `renameat(vault.new, vault)`.
- Parent-directory fsync: the vault directory is fsynced after the
  rename; opening the directory with creation fsyncs its parent every
  time, also when it already existed (`42de59e`, `2fa98e5`). Creation
  of a first vault links `vault.new` to `vault` (fails if one exists),
  removes `vault.new` and syncs the directory.
- Cancellation-independent completion: `Store::start` takes the vault
  out of its slot and runs a `Job` on `spawn_blocking`; nobody awaits
  it. `Job::drop` always puts the vault back and then publishes the
  outcome, also on unwinding or when dropped unstarted. The job holds
  the installation weakly. It writes again while changes arrived after
  its snapshot (bounded at `WRITES_PER_JOB = 4` while a waiter is left).
- Waiters: `Store::wait` subscribes, reads the failure before what is
  durable (`fda2a25`), returns once `durable >= generation`, and starts
  a write for changes left over. `Slot.covered` records what the last
  job wrote as it puts the vault back, so a wait in between starts no
  second write (`6225b3d`). `wait_durable` of an identity fails if the
  identity was closed before or after the wait.
- Failure handling: a failed write keeps the last good generation as the
  durable one, withdraws every session of the installation before any
  waiter is woken, fails every wait, and refuses every later change
  (`check_failed`, `check_open`): the installation fails closed (P4-4).
- Consistent snapshots: a write reads the generation, then the contents;
  `LocalIdentity::snapshot` holds keys, settings and invitations, reads
  the rotation id, then takes `ContactStore::snapshot`, which holds the
  map lock across every slot read and keeps only the marks of that
  rotation. Lock order: map before slot, nothing awaited under a lock
  (P4-3). A written-down global lock order beyond this code order was
  not found.

## 9. Crash/restart semantics

The in-memory directory `MemoryDir` counts every step (create, first
half of a write, second half, file flush, rename, link, remove,
directory sync); `crash_at(step)` fails that step and all later ones;
`restart` builds what a new process finds under four outcomes (unsynced
names kept or lost, torn contents kept or lost).

- Vault: `a_write_stopped_at_any_step_leaves_the_old_or_the_new_vault`,
  `a_creation_stopped_at_any_step_leaves_no_vault_or_a_complete_one`,
  `a_damaged_vault_next_to_a_complete_new_one_is_left_for_the_user`.
- Store (`tests/store.rs`, `crash_everywhere`: every step, every
  outcome; the state after restart is the one before or after, never a
  lower epoch, never a retired key active again):
  - contact mutations: `creating_a_contact_is_atomic`,
    `updating_a_card_is_atomic`;
  - successor promotion: `promoting_a_successor_is_atomic`,
    `an_announced_successor_is_atomic`;
  - deletion and block: `deleting_is_atomic`, `blocking_is_atomic`;
  - invitations: `creating_and_revoking_an_invitation_are_atomic`;
  - rotation: `beginning_a_rotation_is_atomic`;
  - identities: `creating_an_identity_is_atomic`.
- Not covered by a crash-at-every-step test: the switch and the end of a
  rotation and the local mark of an announcement. They have hold and
  restart tests (section 10).
- Persistence failures: `a_failed_installation_admits_nobody`,
  `a_failed_write_withdraws_every_session_even_when_nobody_waits`, and
  the unit tests of `persist.rs` (among them
  `a_failed_write_keeps_the_vault_and_fails_every_wait`,
  `a_write_that_failed_after_one_that_succeeded_keeps_its_success`,
  `nothing_is_written_once_a_write_failed_and_sessions_go_first`,
  `a_cancelled_wait_loses_neither_the_vault_nor_the_outcome`).
- Restart/reopen: `a_restart_reproduces_the_durable_state_exactly`,
  `a_restart_on_disk_reproduces_the_durable_state`,
  `closing_an_installation_lets_go_of_its_directory_at_once`,
  `what_a_refused_dial_recorded_survives_a_restart` and the three after
  it, `the_progress_of_a_rotation_survives_a_restart_with_it_and_no_further`,
  `a_new_identity_is_listed_only_once_it_is_durable`,
  `an_invitation_is_handed_out_only_once_it_is_durable`. Sessions do not
  survive a restart (P4-5).
- On real nodes: steps 5, 6 and 8 of the private Tor network test
  restart node B over its vault (section 19).

## 10. Rotation

- `RotationId { instance, number }`: the instance is the random number
  of the identity instance; `next_rotation` numbers the rotations.
- Announcement binding: an `Announcement` (card, rotation, peer,
  withdrawal, contact instance) is made only by `announcement_for`,
  which requires the session admitted by this store, the identity open,
  the session on the old key, a durable beginning, an accepted contact
  not yet announced and the session still standing. `Sent` is made only
  by `Link::announce` after the EndpointUpdate is written on that same
  link (`LinkError::OtherSession` otherwise). `mark_announced(&Sent)`
  records nothing for another rotation (checked with the keys held), and
  `mark_announced_on` checks the contact instance and `stands_for`.
- Contact-instance binding: `ContactRecord.instance`, `announced` and
  `promoted` as `Option<RotationId>`; a confirmation of the new key is
  read and marked in one step with the keys held (`b3767ba`).
- Switch prerequisites: `switch_rotation(RotationId, force)` does
  nothing for another rotation; without `force` every accepted contact
  must be announced (checked with the map lock held). The new key
  answers and dials only once the switch is durable.
- Finish prerequisites: `finish_rotation` requires every accepted
  contact promoted (or `force`) and a durable switch (`e430064`).
- Durability ordering: the beginning is durable before the successor
  card is handed out (`successor_card` is `None` until then); switch
  before finish; marks committed.
- Stale completion handling: a stale `Sent`, or a forced switch or end
  for another rotation, records nothing.
- Snapshot coherence: the rotation id is read under the keys lock and
  only the marks of that rotation are stored.
- After restart: a restored rotation gets a fresh `RotationId` (new
  instance, number 0) and counts as begun and switched as stored; stored
  marks are rebound to it; contact records get new instance numbers; a
  rotation whose new key equals the old one is refused.
- Tests: `a_rotation_ends_only_after_a_durable_switch`,
  `a_rotation_switches_and_finishes_only_when_due`,
  `the_new_key_reaches_no_peer_before_it_is_durable`,
  `the_progress_of_one_rotation_never_counts_for_the_next`,
  `a_late_announcement_counts_only_for_the_contact_it_was_made_to`,
  `a_late_announcement_counts_for_nothing_once_its_contact_or_session_changed`,
  `a_decision_for_one_rotation_changes_nothing_in_the_next`,
  `an_announcement_is_sent_only_on_its_own_session`, the snapshot tests
  of `contacts.rs` and `identity.rs`, and step 6 of the private Tor
  network test.
- Left on purpose (`docs/DESIGN_QUESTIONS.md` 11.5): `dial_plan` reads
  the "successor sent" mark before it is durable; a crash loses only the
  record and a later session announces again.

## 11. Message 3 / local identity disclosure

- Who may receive it: only a peer whose standing is `Requested` or
  `Accepted` (`may_learn_local_identity`). `Message3` is made only by
  `OutboundPeer::admit` (P4-2); any other standing is refused without
  it. A source scan checks this
  (`only_the_contact_store_admits_a_responder_and_only_dial_writes_message_3`).
- Durability: `dial` admits, takes the contact budget slot, waits until
  what the admission recorded is durable (`wait_durable`, which also
  fails for a closed identity), and only then writes message 3. A
  refused dial also makes what it recorded durable first.
- Withdrawal checks: `write_unless_withdrawn` checks the withdrawal
  before every write, the first one included; on a withdrawal it writes
  nothing more (no Close after possibly partial bytes) and the dial
  fails with `LinkError::Withdrawn`.
- Short writes: a partial write advances and the loop continues; a write
  of 0 bytes or an error ends the link (`LinkError::Stream`). The whole
  dial is bounded by `HANDSHAKE_TIMEOUT`.
- Authorization lost while writing: the rest is not written, the dial
  fails and no session is established. Tests:
  `a_key_retired_while_the_admission_is_made_durable_gets_no_message_3`,
  `a_contact_deleted_or_blocked_during_the_dial_gets_no_message_3`,
  `a_dial_without_a_contact_slot_writes_no_message_3`,
  `a_withdrawal_ends_a_write_of_message_3_that_is_pending`,
  `nothing_is_written_after_a_withdrawal_that_comes_between_two_writes`.
- Bytes already accepted by the stream cannot be recalled: stated in
  `docs/PROTOCOL.md` 4.4 and in S51 (Residual): the first 48 bytes carry
  the local transport key, and message 3 goes in one write in practice
  (235 bytes).

## 12. Session lifecycle

- Registration: the `Withdrawal` is bound to the store's owner at
  admission and tracked only for a contact record whose card stands, in
  the same locked step (S50). Strangers are not tracked.
- Cancellation: a dial or answer dropped while its admission is made
  durable ends its withdrawal (`EndIfCancelled`); ended links are
  forgotten by the next reconcile
  (`a_dial_cancelled_while_its_admission_is_made_durable_leaves_no_session`).
- Retirement: after every change `reconcile` withdraws the tracked
  sessions that no longer stand (P4-5).
- Promotion: the old key is retired and its sessions are withdrawn
  (`a_rotation_withdraws_the_link_of_the_retired_key`).
- Block/delete: record set, then reconcile
  (`blocking_or_deleting_a_contact_withdraws_its_open_links`).
- Identity/installation close: the store is closed and every tracked
  session withdrawn; a failed write withdraws everything. A stranger's
  link is not withdrawn at the closing (left on purpose, section 25).
- Partial send cancellation: a send dropped while writing leaves the
  link terminal; it ends without a Close
  (`a_send_dropped_while_it_writes_leaves_the_link_terminal`).
- Receive cancellation: a message already taken is delivered by the next
  receive; the one after fails
  (`a_message_taken_before_a_receive_was_dropped_is_still_delivered`).
- Terminal behavior: dropping a `Link` ends its withdrawal and every
  later call fails. `Arrived` is made only by `Link::receive` and is
  applied at most once (`StoreError::AlreadyApplied`; the mark is set
  before the commit is awaited, so a cancelled `apply` counts as done);
  `apply` refuses an `Arrived` of another store (`OtherIdentity`) and a
  closed identity; contact effects are rechecked with `stands_for`,
  which refuses withdrawn and ended links. `Sent` (section 10).
  `LinkError::NoEndpoint` for a card the dial plan does not name, before
  any stream.

## 13. Identity and installation lifecycle

- Explicit deletion: `Installation::delete_identity(&LocalIdentity)`
  matches the instance (pointer equality), closes it, removes it and
  commits; a stale handle gets `NotFound` (`fc07b65`,
  `a_deletion_names_the_instance_it_deletes`).
- Installation closure: dropping the last `Installation` handle closes
  every identity as a deletion does; identity handles hold the
  installation weakly; a write under way still ends (`cc8650f`,
  `closing_the_installation_ends_the_authority_of_what_it_handed_out`,
  `a_write_under_way_when_the_installation_closes_still_ends`,
  `closing_the_installation_takes_its_services_down`).
- Closing an identity: closes its store and withdraws its sessions,
  erases the seed and the transport secret, drops the onion key, drops
  invitations and requests, keeps the party it answered with.
- Retained handles: a kept handle has no authority after the close; a
  closed identity still shows the card it answered with (`85d6a33`).
- Credential-bearing getters: there is no public party getter;
  `answering_party` and `dial_party` are crate-private and check the
  closing with the keys lock held; `onion_secret` takes the keys lock,
  then checks (`c9b31ff`); `card_with` and `invitation_card` check the
  closing (`a_deleted_identity_hands_out_no_secret`).
- Onion key: gone after the close; the supervisor stops.
- Invitations: emptied at the close; no card is handed out after it.
- Parties/dial plans: `DialPlan` holds the local card and the peer
  cards, never a party (`4c8e14b`, `eb6cbf4`); `link::dial` takes the
  party from the identity.
- Live-session withdrawal: as section 12.
- Stale asynchronous work: `wait_durable`, `dial` (after its slot and
  after connecting), `consider` and `apply` recheck the closing
  (`a_session_that_outlives_its_deleted_identity_changes_nothing`,
  `a_request_considered_as_the_identity_is_deleted_is_not_queued`).
- Left on purpose: a wait of an identity being deleted fails for a
  change that was made durable (a deleted identity reports nothing as
  done).

## 14. Multi-identity and instance isolation

Every identity instance draws a random 8-byte instance number (at
creation, restore and vault open); `docs/ARCHITECTURE.md` 1.1 keeps
apart logical identity, lifecycle instance, durable record and runtime
handle (S40).

- Session A mutating identity B: `apply` refuses an `Arrived` whose
  withdrawal was not admitted by this store (`OtherIdentity`; T-MI-10,
  CS63).
- Invitation ids: `InvitationId { instance, number }`, compared whole
  (`b3a7cb5`; T-MI-11, also after delete and restore; CS96).
- Request ids: `RequestId { instance, number }`; an answer is for the
  request it was given for (`1314b9d`;
  `an_answer_is_for_the_request_it_was_given_for`; CS99).
- Rotation ids: `RotationId { instance, number }`; switch and end take
  it (`c73482e`; CS101, CS102;
  `the_rotation_of_one_identity_counts_for_no_other`, CS87).
- Contact replacement: a per-store contact instance carried by
  `Announcement` and checked by the marks
  (`a_confirmation_counts_for_no_contact_made_again_meanwhile`,
  `a_message_of_an_ended_link_changes_no_contact_made_again`; `8c0aff1`,
  `b3767ba`, `049de25`).
- Lifecycle aliasing: deletion matches the object, not the keys.
- Tests T-MI-1 to T-MI-11 (`tests/identities.rs`, `tests/link.rs`);
  step 8 of the private Tor network test.
- Open, not Phase 4: MI-1 (target port per identity on Tails and
  Whonix), MI-4 (which phase offers several identities in an
  interface).

## 15. Invitations and requests

- `MAX_ACTIVE_INVITATIONS = 16` (`limits.rs`); a seventeenth is refused
  (`Full`), nothing is evicted (P4-12).
- Persistence before export: an invitation card is signed and handed
  out only once the capability is durable and still active (`2230ad9`,
  `3d4aa84`, `3a6a823`; `an_invitation_is_handed_out_only_once_it_is_durable`,
  `an_invitation_revoked_as_it_is_created_hands_out_no_card`).
- Reusable until revoked: no expiry, no use count (T-INV-4).
- Identity/lifecycle binding: through `InvitationId.instance`
  (section 14).
- Request queue: in memory only; one pending request per sender; at most
  `MAX_PENDING_REQUESTS_PER_INVITATION = 8` per capability and
  `MAX_PENDING_CONTACT_REQUESTS = 32` in all; insertion order; nothing
  evicted; blocked, declined and contact senders dropped.
- Accept/decline/block failures: the answer runs with the queue locked
  and removes the request only on success
  (`a_request_stays_pending_when_answering_it_fails`); a request no
  longer waiting is `NotFound`.
- Races with import/block: both hold the queue lock across the record
  change (`91a3415`, `780c3dc`;
  `a_block_or_an_import_takes_out_no_request_queued_after_it`,
  `a_request_and_a_block_of_its_sender_run_in_one_order`).
- Revoked capability: a request carrying it is dropped; revoking leaves
  contacts and pending requests alone; `revoke_and_discard` also drops
  the requests it admitted (T-INV-7, T-INV-11).
- `Arrived` counts once for requests too (`what_arrived_counts_once`).
- Tests T-INV-1 to T-INV-11 (`tests/invitations.rs`); step 8 of the
  private Tor network test.

## 16. Unknown/silent connection budgets

- Limits (`crates/monolith-protocol/src/limits.rs`):
  `MAX_UNKNOWN_SESSIONS` 4 and `MAX_INBOUND_HANDSHAKES` 16 per identity;
  `MAX_CONTACT_SESSIONS` 256 per identity; process-wide
  `MAX_PROCESS_INBOUND_HANDSHAKES` 32, `MAX_PROCESS_CONTACT_SESSIONS`
  512, `MAX_CONCURRENT_DIALS` 4; `MAX_LOCAL_IDENTITIES` 8;
  `UNKNOWN_SESSION_TIMEOUT` 20 s; `UNKNOWN_FIRST_MESSAGE_TIMEOUT` 10 s;
  `ACCEPT_BACKOFF` 250 ms.
- Unknown: an inbound session whose standing is not a requested or
  accepted contact; outbound sessions are never unknown. Silent: no
  complete message taken yet.
- Deterministic eviction: a newcomer evicts the first held slot that is
  still silent (slots numbered under one lock); the first message and
  the eviction race on one atomic.
- Draining limit: when as many evicted strangers drain as there are
  slots, a newcomer gets no slot: at most 2 x 4 stranger links per
  identity.
- Authenticated contacts are never evicted: they take a contact permit,
  not a stranger slot (`a_contact_is_never_evicted_for_a_stranger`).
- Release: dropping a slot releases it, held or draining; a cancelled
  answer drops its slot and ends its withdrawal.
- Tests: unit tests of `strangers.rs`,
  `a_full_stranger_budget_evicts_the_oldest_silent_stranger`,
  `a_stranger_finds_no_slot_while_evicted_strangers_drain`,
  `a_stranger_evicted_as_its_first_message_is_taken_gets_nothing_delivered`,
  T-MI-7.
- Finding, not Phase 4 code: `docs/RESOURCE_LIMITS.md` (paragraph "When
  the handshake budget is full") says the oldest unauthenticated stream
  is closed to make room for a new one; the code
  (`crates/monolith-core/src/budget.rs`, accept loop) drops the new
  stream and pauses for `ACCEPT_BACKOFF`, and its module doc says so.
  The text dates from `d507ad4` and the behavior from `4a4e4b5`, both
  before `2f27dcb`. [OWNER DECISION: correct the document, or change the
  behavior; recorded as an open point, not fixed during the freeze.]

## 17. Tor supervisor and publication lifecycle

- Control loss: the accept loop races the listener against the closing
  of the control connection and reports `ControlLost`; the supervisor
  reports the service unavailable and starts again.
- Reconnect/backoff: `RECONNECT_DELAY_INITIAL` 10 s, doubling, up to
  `RECONNECT_DELAY_MAX` 30 min, jitter 50%; reset to the first delay
  after `RECONNECT_RESET_AFTER` 60 s of serving.
- Tor process restart: tested with a mock
  (`a_lost_service_is_published_again_under_the_same_name`) and with a
  real Tor in step 9 of the private network test (section 19).
- Onion re-publication: `ADD_ONION` with the existing secret and the
  expected ServiceID; a different ServiceID is refused
  (`OnionKeyMismatch`). Close sends `DEL_ONION` (5 s), then drops the
  control connection. `Detach` is never sent.
- Deletion during publication: a publication is raced against the
  closing and dropped; a service returned for a closed identity is
  closed and not reported; waits between attempts stop at the closing
  (`493dcb7`; `a_deleted_identity_is_taken_down_and_not_published_again`).
- Lifecycle checks: the supervisor uses the closing of the identity
  instance (`closing()`, `is_closed()`), not a generation number. An
  identity without an onion key gets `Publication::NoKey`.
- Multi-identity separation: one control connection and listener per
  publication (S43;
  `the_supervisors_of_two_identities_do_not_touch_each_other`).
- No hot loop: `the_delays_follow_the_reconnect_schedule` (every delay
  at least half the first one) and
  `while_tor_cannot_publish_the_attempts_slow_down` (5 to 12 failures
  per hour, gaps of at least 5 s).

## 18. Fail-closed networking

- `tests/network/fail-closed.sh` (T-NET-1), run in stage 1 on
  `970316f`: passed. Every step runs under `strace` and fails on any
  datagram socket of AF_INET, AF_INET6, AF_PACKET or AF_NETLINK, on any
  port 53, and on any `connect` that is not to a Unix socket or
  loopback. Steps:
  1. SOCKS available: the onion name goes to the proxy, nothing else is
     contacted.
  2. SOCKS unavailable, direct Internet available: the dial fails,
     nothing else is tried.
  3. The CLI with Tor absent prints "offline: Tor is unavailable" and
     exits with 1.
  4. A network namespace with loopback only (no DNS, no route): steps 1
     and 2 again.
- No direct TCP fallback: S1, S2; `connect_onion` maps a connect failure
  to `SocksUnavailable`; the one outbound TCP connect is to loopback.
- No DNS: S3; SOCKS address type 0x03 with the literal onion name;
  `to_socket_addrs` and `lookup_host` banned by `clippy.toml`.
- No HTTP: `deny.toml` bans reqwest, hyper, ureq, curl and isahc (S16,
  S32).
- No UDP: `clippy.toml` bans std and tokio `UdpSocket`; the script fails
  on any datagram socket.
- `TcpStream::connect` and `TcpListener::bind` are banned except for two
  allowed uses in `monolith-tor` (the loopback connect and the loopback
  listener). Only `monolith-tor` enables tokio `net`. Clippy runs with
  `-D warnings` (stage 1, passed).
- Also in the private Tor network test: steps 10 and 11 (section 19).
- Gaps, not Phase 4 work: S32 cites T-NET-1 for "an idle run produces no
  traffic besides Tor's own", but `fail-closed.sh` has no idle step;
  the script uses strace rather than the packet capture that
  `docs/TEST_PLAN.md` T-NET describes. [OWNER DECISION: record or
  close.]

## 19. Private Tor E2E

- Topology: Chutney `ae3a33c`, network `monolith-two-clients`: 3
  authorities (also relays), 5 relays (HSDirs), 2 clients; no public
  relay, no Internet. Debian 12 image (`tests/tor-network/Dockerfile`).
  Tor 0.4.9.13 (from the run's log; the Dockerfile installs `tor` from
  deb.torproject.org without pinning a version).
- Run on `970316f`, 2026-10-08, with `RESTART_CYCLES=15` and the default
  `RECOVERY_WINDOW` of 240 s: all steps passed, "Private Tor network
  test passed", exit 0 (`verification/970316f/stage2/tor.log`).
- Steps (numbered 1, 2, 4 to 12; step 12 is the former Phase 3 step 3):
  1. Publication, SOCKS connection, Noise XK, one message each way,
     DEL_ONION.
  2. Published again from the key in memory under the same name and
     reached twice; a dial without SOCKS fails.
  4. Two nodes over vaults: identities, cards imported, accepted through
     the protocol, hello and hello back.
  5. B restarts: contact, identity and service as before.
  6. A rotates: B keeps the authorized successor across a restart,
     promotes it, withdraws the old session, keeps the retired key
     across a restart.
  7. B blocks A during a session: the session ends, A is a stranger.
  8. A second identity at B: invitation mode, a capability, identities
     kept apart, deletion withdraws the session.
  9. Real Tor B process restart, 15 cycles: the supervisor publishes
     both services again under the same names; Noise XK and hello again.
  10. Without SOCKS, and without Tor while direct Internet exists: both
      dials fail at once.
  11. Every node run contacted only loopback and Unix sockets, no DNS
      (7 node runs checked).
  12. Tor B goes away while B waits: control connection reported lost.
- Bounded recovery (step 9): per cycle, a new session must be made and
  answered within 240 s (two Tor `SocksTimeout`s of 120 s). A send is
  repeated only when the dial fails with "Tor SOCKS endpoint refused the
  request"; any other failure or the end of the window fails the test.
  Cycles start 65 s apart, longer than `RECONNECT_RESET_AFTER`, so every
  restart meets the first backoff delay. The SOCKS CONNECT reply codes
  are read from the strace of node A (the fourth read on a connection to
  Tor A's SOCKS port). No change of the production SOCKS semantics
  (owner decision).
- Observed on `970316f`: 15 of 15 cycles recovered with 1 attempt, 0
  dial retries, SOCKS reply 0x00 every time, in 1 to 20 s (4, 6, 6, 6,
  7, 4, 4, 6, 4, 1, 3, 3, 10, 4, 20).

## 20. Toolchain matrix

Stage 1 on `970316f`, 2026-10-08:

| Toolchain | Command | Binaries | Passed | Failed | Ignored |
| --- | --- | --- | --- | --- | --- |
| 1.95.0 (pinned) | `cargo test --workspace --locked` | 33 | 682 | 0 | 2 |
| 1.85.1 (MSRV) | `cargo +1.85.1 check --workspace --all-targets --locked` | clean | | | |
| 1.85.1 (MSRV) | `cargo +1.85.1 test --workspace --locked` | 33 | 682 | 0 | 2 |
| stable | `cargo +stable test --workspace --locked` | 33 | 682 | 0 | 2 |

Stable was rustc 1.95.0 (59807616e 2026-04-14) that day, the same as
the pinned toolchain. Nightly for the fuzz targets: cargo 1.98.0-nightly
(0b1123a48 2026-06-01).

The two ignored tests: `a_service_published_again_from_its_key_is_reached_again`
(needs the private Tor network; run by step 2 of section 19) and
`unlock_times_at_the_candidate_settings` (a measurement, run with
`--release`).

## 21. Static/tooling checks

Stage 1 on `970316f`:

- `cargo fmt --all --check`: clean.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`:
  clean.
- `cargo deny check`: advisories ok, bans ok, licenses ok, sources ok.
- `cargo audit`: 82 crate dependencies scanned against 1294 advisories,
  no vulnerability.
- `cargo tree -d`: only `rand_core` 0.9.5 (through `proptest`, a
  dev-dependency) and 0.10.1 (through `x25519-dalek`), as recorded in
  `deny.toml`.
- `cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins
  --locked`: clean.
- Dependency/security notes: Phase 4 added 11 crates to `Cargo.lock`
  (`argon2`, `base64ct`, `blake2`, `errno`, `hkdf`, `linux-raw-sys`,
  `password-hash`, `phc`, `rustix`, `tinyvec`,
  `unicode-normalization`), for the vault (`docs/STORAGE.md`,
  `docs/DEPENDENCIES.md`). No manifest or lock file changed after
  `8b6a6b8`, where the first stage also passed with 82 crates.

## 22. Regression test suite

- 33 test binaries; 682 passed; 0 failed; 2 ignored (section 20), on
  each of the three toolchains.
- Deterministic regression tests from the review closure (each failed
  on the code before its fix; the concurrency ones are made
  deterministic by `#[cfg(test)]` hooks), by area:
  - Lifecycle and deletion:
    `closing_the_installation_ends_the_authority_of_what_it_handed_out`,
    `a_deletion_names_the_instance_it_deletes`,
    `a_deleted_identity_hands_out_no_secret`,
    `a_session_that_outlives_its_deleted_identity_changes_nothing`,
    `a_closed_identity_shows_the_card_it_answered_with`,
    `a_deleted_identity_is_taken_down_and_not_published_again`,
    `t_mi_11_an_invitation_handle_of_one_identity_is_refused_by_another`.
  - Rotation and announcements:
    `the_progress_of_one_rotation_never_counts_for_the_next`,
    `a_decision_for_one_rotation_changes_nothing_in_the_next`,
    `a_late_announcement_counts_for_nothing_once_its_contact_or_session_changed`,
    `an_announcement_is_sent_only_on_its_own_session`,
    `a_confirmation_counts_for_no_contact_made_again_meanwhile`,
    `a_snapshot_is_one_cut_while_a_rotation_changes`.
  - Dial and admission:
    `a_card_the_dial_plan_does_not_name_is_not_dialed`,
    `a_dial_cancelled_while_its_admission_is_made_durable_leaves_no_session`,
    `a_dial_without_a_contact_slot_writes_no_message_3`,
    `a_message_of_an_ended_link_changes_no_contact_made_again`.
  - Requests and invitations:
    `an_answer_is_for_the_request_it_was_given_for`,
    `what_arrived_counts_once`,
    `a_block_or_an_import_takes_out_no_request_queued_after_it`,
    `an_invitation_revoked_as_it_is_created_hands_out_no_card`.
  - Durability:
    `a_failed_write_withdraws_every_session_even_when_nobody_waits`,
    `a_wait_sees_what_a_failing_job_made_durable`,
    `a_wait_finds_the_vault_back_once_its_outcome_is_out`,
    `nothing_is_written_once_a_write_failed_and_sessions_go_first`,
    `closing_an_installation_lets_go_of_its_directory_at_once`.
  - Link and concurrency hooks:
    `nothing_is_written_after_a_withdrawal_that_comes_between_two_writes`,
    `a_send_dropped_while_it_writes_leaves_the_link_terminal`,
    `a_stranger_evicted_as_its_first_message_is_taken_gets_nothing_delivered`,
    `no_operation_runs_between_the_reads_of_a_snapshot`,
    `no_snapshot_runs_between_the_two_slots_of_a_decline`.
  - Structure scans (`tests/structure.rs`):
    `no_party_of_a_local_identity_leaves_the_core`,
    `only_what_a_link_received_is_applied`,
    `only_a_sent_announcement_counts`.
- Known weakness: `what_arrived_counts_once` checks that a second
  application fails, not that it fails with `AlreadyApplied`.

## 23. Fuzzing

- The 16 fuzz targets (`fuzz/Cargo.toml`, 16 `[[bin]]` entries):
  `base32`, `contact_card`, `contact_card_text`, `contact_store`,
  `credential_sequence`, `frame_plaintext`, `frame_stream`,
  `handshake_initiator`, `handshake_responder`, `message_body`,
  `session_frames`, `session_sequence`, `socks_reply`, `text_fields`,
  `tor_control_reply`, `vault_payload`.
- `fuzz/fuzz_targets/session_fixtures.rs` is a module the session
  targets share, not a seventeenth target. No target was removed.
- Smoke run on `970316f` (2026-10-08): all 16 targets, 25 s each
  (`-max_total_time=25 -malloc_limit_mb=64 -timeout=5`, corpus and
  seeds): exit 0, no crash, for every target.
- Long runs on `970316f`: 600 s each of `credential_sequence`,
  `contact_store`, `vault_payload`, `session_frames`,
  `handshake_initiator`, `handshake_responder`, `tor_control_reply` and
  `socks_reply`: exit 0, no crash (`verification/970316f/stage2/fuzz-summary.txt`
  has the run counts).
- Corpus/crash status: corpora and artifacts are ignored by git. No new
  artifact was written by the final runs. One older artifact exists
  locally, `fuzz/artifacts/contact_store/crash-ed9adc4f...` (6 bytes,
  2026-10-05, before the freeze); run against `970316f` on 2026-10-09 it
  does not crash.

## 24. Mutation verification

Expected manifest at `970316f` (`mutation/README.md`,
`mutation/faults.py`, counted per list): 338 faults; phase 1: 45,
phase 2: 99, phase 3: 71, phase 4: 123 (`CS1` to `CS123`); expected
329 caught, 9 survivors (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29),
0 invalid. Phase 4 expects zero survivors: no Phase 4 entry is
expected to survive.

How it ran: `python3 mutation/run.py <phase> 4`, one phase at a time,
alone, timeout 1200 s per fault. Phases 1 and 2 ran on 2026-10-08 on
the development machine (12 threads, the runs kept on 11 of them).
Phases 3 and 4 run [PENDING: date, machine, worker count] with
`verification/970316f/run-phases.sh` in a detached worktree of
`970316f`. STATUS.md 5.2 names one `all` run; the run was split by
phase so that each phase's results are written when it ends; the faults
and the copies are the same.

| Phase | Faults | Caught | Survived | Invalid | Unexpected | Runner exit |
| --- | --- | --- | --- | --- | --- | --- |
| 1 | 45 | 43 | B3, B4 | 0 | 0 | 0 |
| 2 | 99 | 95 | CAP2, CAP3, S15, S24 | 0 | 0 | 0 |
| 3 | 71 | [PENDING] | [PENDING] (expected Q14, Q18, Q29) | [PENDING] | [PENDING] | [PENDING] |
| 4 | 123 | [PENDING] | [PENDING] (expected none) | [PENDING] | [PENDING] | [PENDING] |
| Total | 338 | [PENDING] | [PENDING] | [PENDING] | [PENDING] | |

- Faults caught only by the runner's timeout: phase 1 none. Phase 2:
  T6 (the Montgomery comparison inverted) reached the 1200 s timeout
  while four workers shared the CPUs; run alone on the same commit it
  was caught in 791 s by failing tests (dozens: card decoding, dial,
  admission), not by the timeout
  (`verification/970316f/stage3/results-phase2-970316f-T6.json`).
  Phases 3 and 4: [PENDING: list every result whose `failed` holds
  `(timeout)`, and the outcome of running each alone]. CS30 is expected
  to be caught by the timeout with two failing supervisor tests
  (`a_lost_service_is_published_again_under_the_same_name`,
  `the_supervisors_of_two_identities_do_not_touch_each_other`): with no
  backoff delay the supervisor loops and the run does not end.
- Concurrency faults CS28, CS51, CS52 are deterministic: each test puts
  a `#[cfg(test)]` hook at the point of the race (CS28: a stranger
  evicted between taking its first message and delivering it; CS51: a
  check at every slot read of a snapshot that the map lock is held,
  whatever the order of the map; CS52: a snapshot attempted between the
  two slots of a decline). The hook tests passed 20 times out of 20
  under load before the freeze.
- Kill mechanisms that deserve a note: CS17 is caught at once by
  `a_dial_without_a_contact_slot_writes_no_message_3` and
  `what_a_dial_without_a_slot_recorded_survives_a_restart` since
  `c413a9f` (no longer by the timeout); K4 is caught by a doctest, so no
  test name is recorded; T6 as above.
- The runner counts a fault as caught when the tests fail or time out,
  survived when they pass, invalid when the tree does not compile; it
  exits 0 only if every outcome matches its expectation and nothing is
  invalid.
- Mutation testing shows that the suite detects the modeled fault
  classes listed in `mutation/faults.py`. It does not show the absence
  of other bugs.

## 25. Review closure, residual limits and deferred work

- Reviews (`docs/DESIGN_QUESTIONS.md` 11.4 and 11.5, `STATUS.md` 5.1):
  - First independent review of the Phase 4 code: no path gives a
    contact session or message 3 to a key or record that does not
    stand; findings on durability, cancellation and the life of an
    identity, all fixed. A review of those fixes found two open paths
    and two faults that did not work; the runner's baseline found one
    more. All fixed.
  - The owner's review at `e29d7f2`: thirteen findings, all real, each
    reproduced by a test that failed before its fix. A second review of
    the whole Phase 4 code and reviews of its fixes: all fixed.
  - An outside review of `642a5ed`: three findings and two mutation
    weaknesses. The pattern behind them, a logical name taken for an
    object instance, led to a sibling search that found ten more
    (request handles, the instance in `delete_identity`, rotation ids
    in switch and end, the queue lock across import and block, the card
    of a capability revoked as it was made, parties kept in the core,
    the card a closed identity answered with, dials refused for
    non-contacts, the card of a contact request, and three dev-node
    faults). All fixed.
  - Two reviews of that round and two of their fixes: all findings
    fixed (among them `Arrived`, `Sent`, the dial plan check, the wait
    order).
  - The last independent review, of `51973c5..7eff0c4`: no defect in
    the production code. It found a test that killed CS121 only by a
    race and a check of `Sent` that could be dodged (both test or
    structure fixes, `d24ab14`), and that what arrived on a link that
    had ended counted until its record forgot the link; the fix of the
    latter, `d490292` (CS122), changes production code (`stands_for`
    refuses ended links). A focused mutation run afterwards, not a
    review, found a write-job race (`6225b3d`, CS123, production code).
    No review ran after `6225b3d`; the code has not changed since
    (`c413a9f` changes a fault, `b41cdeb` the network test, `970316f` a
    document).
- Left on purpose, with reasons in 11.5: a wait of an identity being
  deleted fails for a change made durable; `dial_plan` reads the
  "successor sent" mark before it is durable; a stranger's link is not
  withdrawn at the closing (it may only send Close; nothing is applied;
  it ends at its first message or after `UNKNOWN_SESSION_TIMEOUT`); the
  check of CS81 in `apply` is a second line.
- Owner decisions: `link::answer` stays bound to its service by `serve`
  (type-level binding is later hardening); the SOCKS semantics of
  `monolith-tor` stay as they are (0x01 is `SocksRefused`); the gap S20
  to S24 in `docs/SECURITY_INVARIANTS.md` stays.
- Not provable by ordinary tests:
  - physical power loss: the crash model covers lost and torn writes
    and unsynced names, not the device; the parent-directory flush has
    no test;
  - erasure of freed memory: best effort with `zeroize`; not observable
    without `unsafe`, which the workspace forbids (CAP2); the Noise
    library does not erase its own state;
  - constant-time comparison: a timing property; no test measures it
    (CAP3);
  - an admission that fails after changing a record: no path reaches
    it.
- Open points found while drafting this report (pre-existing, not
  Phase 4 changes): the handshake budget text of
  `docs/RESOURCE_LIMITS.md` (section 16); the idle-traffic claim of S32
  and the strace-versus-capture difference of T-NET-1 (section 18);
  the Tor version not pinned in the Dockerfile (section 19).
  [OWNER DECISION on each.]
- Deferred, outside Phase 4 (`STATUS.md` 5.5): the session manager with
  the duplicate rule and per-contact rates, the reconnect scheduler,
  the outbound queue and the message store (ST1), the command and event
  interface for a front end, the review of `MaxStreams` (C1), the KDF
  measurement on the four targets of `docs/STORAGE.md` 3.3, a target
  port per identity on Tails and Whonix (MI-1), and the phase that
  offers several identities in an interface (MI-4). None of these is
  part of the Phase 4 result.

## 26. Final Phase 4 verdict

[PENDING: phases 3 and 4 of the mutation run on `970316f`.]

Stages 1 and 2 passed on `970316f`; mutation phases 1 and 2 passed with
the expected survivors only. Until phases 3 and 4 have run and match
their expectations:

    PHASE 4 NOT COMPLETE

If every stage on `970316f` succeeds, this section will read:

    PHASE 4 COMPLETE

with `970316f` as the Phase 4 baseline for future work; its regression
tests and mutation invariants are the preserved security baseline. The
mutation suite detects the modeled fault classes; it does not prove the
absence of all bugs.
