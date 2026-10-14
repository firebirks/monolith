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
| `07d6335` | a test of a message in flight sent once and waited for from then, for MR28, which survived the final verification of `75ccf7c` (R5-30) |
| `75ccf7c`, `91bf22c` | this file, before each final verification |

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

The final verification of `75ccf7c`, from 2026-10-12 08:52, did not close
the phase. Everything passed up to the mutation lists: `cargo fmt`,
clippy with warnings denied; the tests on Rust 1.95.0, 1.85.1 and 1.99.0
(39 binaries, 781 passed, 2 ignored, on each); `cargo tree -d` with only
the known pair of `rand_core`; the fail-closed test; the private Tor
network test with 15 restarts of Tor B (2048 s); `cargo deny`; `cargo
audit` (83 crates, no advisory); the 19 commits after `phase-4-complete`
all by `firebirks`; the 18 fuzz targets for 25 s each and
`message_store`, `message_sequence`, `vault_payload` and `contact_store`
for 600 s each, with no crash. The Phase 5 list then caught 56 of its 57
faults and MR28 survived (`docs/DESIGN_QUESTIONS.md` section 12.7,
R5-30); the lists of Phases 4 to 1 were not run on that commit. On
`07d6335` the official runner caught MR28 with the test written for it,
after an unmodified run of 604 s. No production code, network test,
dependency or fuzz target changed after `63d91f6`, so its dry run of the
private Tor network test stands.

The final verification of `91bf22c` ran from 2026-10-12 16:41 to
2026-10-14 17:37, in full, and passed. Its results are the final report,
section 6.

## 5. What is left, in order

1. The owner's review of the final report (section 6).
2. On the owner's instruction only: whether the outputs of the
   verification go on a branch of their own, as for Phase 4; the push,
   the merge into `main` and the tag. Then this file is removed.
3. The follow-up work of section 6, item 29, in a later phase, or before
   the merge if the owner wants it there. A change before the merge needs
   a new frozen commit and the stages of the verification it affects.

## 6. Final report

Written on 2026-10-14, after the final verification of `91bf22c`. No
result here is inferred: each comes from an output of that verification,
named where it is used, or from the repository at `91bf22c`. The outputs
are kept on the machine that ran it (item 2); they are not in the
repository.

### 1. Phase 4 baseline

- Tag `phase-4-complete` (tag object `a2bf157`) on `3c013f5`, which is
  also `main` and `origin/main`. Its production code is that of
  `970316f`, the commit on which Phase 4 was verified and which the owner
  declared complete on 2026-10-11.
- What Phase 5 had to keep: the tests of Phases 1 to 4, and the mutation
  lists `PHASE1` to `PHASE4` of `mutation/faults.py` (45, 99, 71 and 123
  faults) with their outcomes on `970316f`: every fault caught except
  nine, each documented as equivalent, not observable or unreachable (B3,
  B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29).

### 2. Verified commit

- `91bf22cd60930e52f53229bde389e8f115927e64` ("Record the final
  verification of 75ccf7c and the fix of MR28"), on
  `phase-5-messaging-runtime`, committed on 2026-10-12 at 16:39 +02:00.
- Every result of items 21 to 25 and 27 was produced on this commit, with
  a clean tree:
  - container stage: an export of the tree of `91bf22c` without `.git`,
    marked with its hash; each of its 362 files was checked byte for byte
    against the commit;
  - WSL stage: a new clone, checked out at `91bf22c`;
  - mutation lists: copies made by `mutation/run.py` with `git archive`
    from a clone checked out at `91bf22c`, each copy checked before its
    first fault (a known fault is caught, and restored it builds again and
    passes).
- No file of the repository changed between the freeze and the end of the
  verification. The commit that adds this report changes `STATUS.md`
  only.
- Outputs: `~/verification-91bf22c` in the WSL Debian of the machine
  that ran the verification (137 files, 4.7 MB): `container`, `wsl`,
  `mutation` (every log, every result file, the records of the pauses),
  `alone` (item 24), `first-75ccf7c` and the scripts that ran them. For
  Phase 4 the outputs went on a branch of their own; whether these do is
  the owner's decision (section 5).

### 3. Branch and history

- Branch `phase-5-messaging-runtime`, local only. Nothing was pushed,
  merged or tagged; `main` is still `3c013f5`.
- `phase-4-complete` is an ancestor of `91bf22c` (`git merge-base
  --is-ancestor`); 21 commits lie between them, none a merge. Author and
  committer of all 21: `firebirks
  <336470935+firebirks@users.noreply.github.com>`; no trailers. No commit
  of Phases 1 to 4 was rewritten.
- The commits are listed in section 3. Two of them are freezes:
  - `75ccf7c`, the first. Its final verification passed every stage up to
    the mutation lists; then the Phase 5 list caught 56 of 57 faults and
    MR28 survived (R5-30, item 26). The phase could not close on it.
  - `91bf22c`, the second, after `07d6335` added the test for MR28. This
    report is its verification.

### 4. Architecture added

No new external dependency (`Cargo.lock` adds only the workspace crate
`monolith-runtime`; `proptest` was already a dev-dependency).

- `monolith-storage`: vault payload version 2 with durable identifiers
  (`record.rs`: `IdentityId`, `ContactId`, `PayloadVersion`); the message
  store file (`messages.rs`) and its payload (`message_records.rs`); the
  names `messages` and `messages.new` in `dir.rs`.
- `monolith-core`:
  - `messages.rs`: the state of the conversations of an installation,
    the state machine of every message, its limits and its reconciliation
    with the vault;
  - `identity.rs`: queueing, taking in, acknowledging and the head of
    each queue, bound to a local identity and its contact records, each
    used only once durable;
  - `persist.rs`: the write job of Phase 4 with a second writer, for the
    message store;
  - `sessions.rs`: the session manager of one local identity;
  - `runtime.rs`: the runtime, which owns the tasks of every link, the
    reconnect schedulers, the commands and the events;
  - `api.rs`: the plain values of commands, queries and events.
- `monolith-runtime` (new): `Messenger`, the boundary of a front end.
- `monolith-cli`: `dev-runtime`, a node on the runtime for the private Tor
  network test.
- Fuzz targets `message_store` and `message_sequence`; property tests
  `tests/message_model.rs` and `tests/runtime_model.rs`; steps 13 to 20 of
  `tests/tor-network/two-node.sh`.
- ADRs 0007 (delivery semantics), 0008 (message store) and 0009 (session
  manager, duplicate sessions, reconnecting).

### 5. Delivery semantics

`docs/adr/0007-delivery-semantics.md`: durable at-least-once transport
and exactly-once local acceptance by deduplication. A message is
`Queued` from the moment it is durable (a successful send means "durably
queued", also without a session or with Tor down) until the
acknowledgement of its identifier, from a session that stands for the
contact record it was queued for, is durable; then `Delivered`, for
good. One queue per conversation, first in first out, one message in
flight; a retry is always the head, so it never reorders later messages.
An "in flight" mark exists only in the task of a session and is not
stored.

### 6. MessageId design

16 bytes from the random source of the operating system (`getrandom`),
drawn when the message is queued and checked by the state under the lock
of the messages. They never change and are never reused. An identifier
that an outbound message of the conversation holds already is refused
(`IdTaken`) and a new one is drawn, at most three more times. The store
refuses one identifier twice in one direction of a conversation. The
receiver deduplicates by identifier per conversation.

### 7. Wire extension

None (P5-2). ChatMessage (a 16-byte identifier and the text) and
MessageAck (the identifier) of `PROTOCOL.md` sections 8.5 and 8.6 already
carry what the runtime needs. Phase 5 changes the text of `PROTOCOL.md`
only: in 8.5 and 8.6 the acknowledgement follows durable storage, the
duplicate check is in the message store and one message is in flight per
contact; in 8.2 a keepalive Ping is optional and Monolith keeps no idle
session; open point P6 says what is ordered. Noise XK, the frames and the
known-answer vectors are unchanged.

### 8. Outbox design

- A send is checked before anything is stored: the text against the
  rules of `ChatText`, the record against the contact store (the current
  record of a requested or accepted contact), the limits of the queue.
- The message enters the state only for a record the vault holds
  durably, and is reported queued, and offered to a session, only once the
  message store holds it (R5-1: `LocalIdentity::next_to_send`).
- A send runs as a task of the runtime: once begun, it completes and
  raises its event also if its caller stops waiting (R5-16), and a
  shutdown waits for it within its deadline (R5-24).
- The queue of a conversation whose contact was blocked or deleted stays
  stored, ended, and is never sent (P5-9).

### 9. Inbox and deduplication design

- A ChatMessage is taken in only from a session that still stands for its
  contact record (ADR 0007); what arrived on a link that ended counts for
  nothing (P5-13).
- The identifier is looked up in the same locked step that inserts the
  message; a duplicate inserts nothing and raises no event.
- The duplicate check covers what the conversation holds and, for
  received messages the user deleted, a window of `MESSAGE_DEDUP_WINDOW`
  (1024) identifiers per conversation.
- It is per contact record: a contact made again starts an empty
  conversation and consults nothing of the old one (S56; R5-7).
- A message that cannot be stored is not acknowledged, and the sender
  keeps it: a limit is reported as `MessageRefused` (reason `StoreFull`),
  a failed write as `StorageFailed`. The reason `RefusedReason::Storage`
  exists in the API but is never raised.

### 10. ACK semantics

- The receiver sends a MessageAck only for what `LocalIdentity::take_in`
  returned, once it is durable (S53); a duplicate is acknowledged again
  once the state that holds it is durable.
- A MessageAck is applied only to the outbound messages of the
  conversation of the record its session stands for (S54). An unknown
  identifier is ignored; a second MessageAck for a delivered message
  changes nothing. `Delivered` is set only in
  `MessageState::acknowledge`, called only from
  `LocalIdentity::acknowledged` for a MessageAck a link received
  (`tests/structure.rs`).
- A written frame, a written Noise message or an established session is
  not delivery. A session that ends with a message in flight reports
  nothing delivered.
- `MESSAGE_ACK_TIMEOUT` (60 s) after the one send of the message in
  flight, the session is closed and the message is sent again on a later
  session (P5-10; R5-30 made the "one send" tested).

### 11. Crash semantics

The table of ADR 0007: a crash of the sender before its message is
durable leaves nothing to resend; after, the message is `Queued` and sent
again; a crash of the receiver before its store is durable means no
acknowledgement was sent; after it and before the acknowledgement, the
retry is found by identifier, stored nothing and acknowledged again; a
crash of the sender between the acknowledgement and a durable
`Delivered` sends once more and is acknowledged again; `Delivered` stays.
A crash at every step of making the store leaves an installation that
opens (R5-12). Covered by `tests/messages.rs` (T-MSG-3), the property
tests with crashes of either side (T-MSG-11), and steps 15 and 16 of the
private network test (item 25).

### 12. Session manager

`docs/adr/0009-session-manager.md`: one manager per local identity
instance, keyed by `ContactId`. Every link runs in one task that owns
it. A session is registered when it reaches `AuthenticatedContact` and
the contact store says, under the lock of the contact, that it stands
for that record (S50). Only the preferred session sends the queue; any
registered session of the record takes messages and acknowledgements in.
A session leaves the manager when its task ends, however it ends,
including an accept loop aborted with its service (R5-8). No keepalive:
a session on which no ChatMessage or MessageAck was sent or received for
`SESSION_LINGER` (120 s), with nothing queued locally for its record,
nothing in flight and no probe running, is closed with Close (P5-7).
`SessionAvailable` and `SessionUnavailable` are raised only when whether
a record has a session changes, decided under one lock (R5-25).

### 13. Duplicate-session resolution

The rule of `PROTOCOL.md` section 14 through `duplicate::resolve`: same
initiator, the newer stays; different initiators, the session initiated
by the smaller identity key is preferred, and if it is the older it is
probed with a Ping with a fresh nonce, kept if the Pong comes within
`DUPLICATE_PROBE_TIMEOUT` (10 s), closed otherwise; a session whose key
is no longer the active credential loses. Both sides choose the same
session, since the choice depends only on the two identity keys and who
initiated each. A message in flight on a closed session stays queued and
is sent again on the one that stays; the receiver drops it by identifier.
Tested in every order (`sessions::tests`, T-MSG-7) and in step 19 of the
network test (both sides sending at once: each message once, one session).

### 14. Reconnect scheduler

One scheduler per conversation with queued messages, a current record,
an open identity and no registered session (P5-8). ADR 0009 also names a
card confirmed for dialing; the code does not wait for one, but a record
without a dialable card is not dialed: each attempt is reported
`DeliveryDeferred` with reason `NoEndpoint`, with the growing delays.
The first attempt follows the send at once; at startup attempts are
spread over `STARTUP_DIAL_SPREAD` (60 s); then 10 s doubling to 30 min,
times a random factor between 0.5 and 1.5. The delays start over only
after a session stayed registered for `RECONNECT_RESET_AFTER` (60 s) and
did not end for want of an acknowledgement, inbound or outbound (R5-9,
R5-23). Every dial is `link::dial` through the Tor backend within
`MAX_CONCURRENT_DIALS` (4); no other path, no name resolution. A contact
with nothing queued is never dialed by the runtime; only
`Runtime::connect`, which `Messenger` does not offer and only tests
call, dials without a queue. The scheduler checks its record and
identity before every attempt and cuts a delay or a dial short when
either ends (R5-3).

### 15. Contact lifecycle

Blocking or deleting a contact ends its record: its sessions are
withdrawn (Phase 4), its scheduler ends at once, its queue stays stored,
ended, and is never sent; a record made again for the same remote
identity has a new `ContactId` and an empty conversation. A new card or a
rotation of the same contact keeps the record and its queue. An attempt
that comes back with a session of another record of the same remote
identity gives it to the manager under that record, with that record's
queue. Step 18 of the network test: with B blocked and back for two
minutes, A made no session, no attempt and no delivery.

### 16. Identity lifecycle

Every local identity has an `IdentityId` that never changes and is never
reused; one restored from its keys gets a new one and starts empty.
Deleting an identity closes it, ends its supervisor, sessions and
schedulers, and drops its messages after the vault no longer holds it;
the check of the identity and the change of the messages are made under
the lock of the messages, so nothing of a deleted identity is added after
(R5-15). The identities of one installation keep messages, queues and
sessions apart (T-MSG-5; step 19 of the network test). A vault of Phase 4
opens: its records get identifiers, and the store is made and read
before the vault is written as version 2.

### 17. Message-store security

`docs/adr/0008-message-store.md`:

- A file `messages` beside the vault, under its lock: header (magic,
  version, generation, 24-byte nonce, length), then XChaCha20-Poly1305
  under a key HKDF-SHA256 derives from the vault key (info
  `MONOLITH-MESSAGE-STORE-V1`), with the header as associated data. The
  key never leaves `monolith-storage`; a passphrase change keeps it.
- The plaintext is padded to a multiple of 64 KiB; the file is at most
  16 MiB and exactly as long as its header says.
- Rewritten whole and replaced atomically as the vault is (`messages.new`,
  flush, rename, flush of the directory), with its own generations and
  write job.
- Read checks everything before use: version, zero padding, every count
  against its limit before allocation, identifiers non-zero and unique,
  sequences, kinds, every text against `ChatText`, the queue limits. A
  store that fails a check or does not authenticate is not opened, and
  the installation is not opened with it; nothing is repaired by
  guessing.
- Matched against the vault at open: a section of an identity the vault
  does not hold is dropped; a conversation of a record that is gone is
  kept, ended; a conversation that names a record of another remote or
  local identity is refused as corrupt. A vault with identities and no
  store opens with an empty store and says so
  (`message_store_missing`); a vault of Phase 4 put back beside a store
  with messages is refused (`OpenError::MessageStore`) (R5-11, R5-27).
- A failed write fails the store closed: nothing more is queued, taken
  in or acknowledged until the process starts again, and `StorageFailed`
  is raised once.
- No message text in `Debug` output (T-MSG-12).
- Not protected (item 28): rollback of the file by someone who can write
  the directory; the size of the file and the times it is written, in
  steps of 64 KiB; data in older copies or freed blocks.

### 18. Resource limits

The limits Phase 5 uses (`docs/RESOURCE_LIMITS.md` sections 2, 4, 5,
5.1, 7, 8 and 9):

| Limit | Value |
| --- | --- |
| `MAX_CHAT_TEXT_LEN` | 16384 bytes |
| `MAX_QUEUED_MESSAGES_PER_CONTACT` | 256 |
| `MAX_QUEUED_MESSAGES_PER_IDENTITY` | 1024 |
| `MAX_QUEUED_MESSAGE_BYTES` | 8 MiB, all identities (lowered from 64 MiB) |
| `MAX_CONVERSATIONS_PER_IDENTITY` | 2048, ended ones included |
| `MAX_CONVERSATION_STORE_LEN` | 4 MiB |
| `MESSAGE_DEDUP_WINDOW` | 1024 per conversation |
| `MAX_STORED_MESSAGES` | 262144, all conversations |
| `MAX_MESSAGE_STORE_FILE_LEN` | 16 MiB |
| `MESSAGE_STORE_PADDING_BLOCK_LEN` | 64 KiB |
| `MAX_LOCAL_IDENTITIES` | 8 |
| `MESSAGE_ACK_TIMEOUT` | 60 s |
| `SESSION_LINGER` | 120 s |
| `DUPLICATE_PROBE_TIMEOUT` | 10 s |
| `MAX_CONCURRENT_DIALS` | 4 |
| Reconnect delays | 10 s doubling to 30 min, jitter 50 %, reset after 60 s, startup spread 60 s |
| `MAX_UI_EVENTS` | 256 |

A send above a limit fails and stores nothing; a received message above a
limit is not stored and not acknowledged; nothing accepted is dropped to
make room. A state with more identity sections than
`MAX_LOCAL_IDENTITIES`, or a conversation whose sequences ran out, is
refused (R5-17). The per-contact rates of section
6 are not enforced (item 28).

### 19. Command API

`monolith_runtime::Messenger`, over `monolith_core::api`, all plain
values:

- opening: `ephemeral`, `create`, `open` (`OpenError`),
  `message_store_missing`, `shutdown(deadline)`;
- identities: `identities`, `create_identity`, `delete_identity`,
  `own_card`;
- contacts: `contacts`, `add_contact`, `block_contact`, `delete_contact`;
- messages: `conversations`, `messages`, `message`, `send_message`
  (`SendError`), `delete_conversation`;
- events: `events`.

Identities, records and messages are named by `IdentityId`, `ContactId`
and `MessageRef` (identity, record, sequence), which stay the same across
restarts. Nothing that crosses the boundary is a local identity,
installation, link, session, key, capability or handle of the contact
store (P5-12, checked by `tests/structure.rs`). Commands that change
something run as tasks of the runtime and complete once begun. Contact
requests and invitations are in the core (Phase 4) but not yet in this
API (item 29).

### 20. Event API

`Messenger::events` returns a receiver of a bounded broadcast of
`MAX_UI_EVENTS` (P5-11). Events: `MessageQueued`, `MessageReceived`,
`MessageDelivered`, `MessageRefused`, `DeliveryDeferred` (with the
reason and the delay before the next attempt), `SessionAvailable`,
`SessionUnavailable`, `Service`, `ContactAccepted`, `RequestReceived`,
`IdentityClosed`, `StorageFailed`. None carries message text. An event
that reports stored state (a message queued, received or delivered, a
contact accepted) is raised after that state is durable. `Service`,
`SessionAvailable` and `SessionUnavailable` report state of the runtime
that is not stored, `RequestReceived` a request held in memory, which is
never stored (S28), and `IdentityClosed` is raised when the identity
closes, which for a deletion is before the vault commit; if that commit
fails, `StorageFailed` follows and a restart shows the identity. ADR
0007 and the documentation of `api::Event` say that every event follows
durable state; that wording is too broad (item 29). A consumer that
falls behind gets `Missed::Lagged(n)` and reads the state again with
queries; nothing waits for a slow consumer, and no correctness property
depends on an event being seen.

### 21. Test totals

On `91bf22c`, each of Rust 1.95.0, 1.85.1 and 1.99.0: 39 test binaries
(31 test targets and 8 doctest runs), 782 passed, 0 failed, 2 ignored
(`tor_network`, which needs the private network, and `kdf_benchmark`, a
measurement). On `970316f` the count was 33 binaries and 682 passed; Phase
5 adds 100 tests and 6 binaries. Covered: T-MSG-1 to T-MSG-13
(`docs/TEST_PLAN.md` section 3b), including the property tests of the
message state and of the runtime against models.

### 22. Toolchain matrix

In the container (`rust:1.95-bookworm`, Debian 12):

| Check | Toolchain | Result | Time |
| --- | --- | --- | --- |
| `cargo fmt --check` | 1.95.0 | pass | 1 s |
| clippy, warnings denied, all targets | 1.95.0 | pass | 24 s |
| workspace tests | 1.95.0 | 782 passed, 0 failed, 2 ignored | 611 s |
| `cargo check` | 1.85.1 | pass | 17 s |
| workspace tests | 1.85.1 | 782 passed, 0 failed, 2 ignored | 628 s |
| workspace tests | stable 1.99.0 | 782 passed, 0 failed, 2 ignored | 604 s |
| `cargo tree -d` | 1.95.0 | only `rand_core` 0.9.5 (proptest, dev) and 0.10.1 (x25519-dalek), as `docs/DEPENDENCIES.md` says | 0 s |

In the WSL clone (Debian 13.4, Rust 1.95.0): `cargo deny` 0.20.2 pass
(advisories, bans, licenses, sources); `cargo audit` 0.22.2, 83 crates,
no advisory.

### 23. Fuzz targets and results

18 targets, 2 of them new (`message_store`, `message_sequence`, each with
seeds committed and checked by the `fuzz_seeds` tests). Nightly 1.101.0
(2026-10-10), cargo-fuzz 0.13.2: the targets built (`cargo check` with
`--cfg fuzzing`, 17 s, then `cargo fuzz build`, 42 s).

- Every target for 25 s: all exit 0, no crash line.
- Long runs, 600 s each, on the new and the affected targets:
  `message_store` 8,453,936 runs, `message_sequence` 95,856,
  `vault_payload` 2,766,196, `contact_store` 259,493. No crash, no
  artifact.

### 24. Mutation totals and results

`mutation/run.py`, 2 workers, the timeout of each list measured from its
unmodified run (twice the workers times that run, at least 20 minutes).

| List | Faults | Caught | Survived | Unmodified run, timeout |
| --- | --- | --- | --- | --- |
| Phase 5 (MR) | 57 | 57 | 0 | 745 s, 2980 s; after the pause 687 s, 2748 s |
| Phase 4 (CS) | 123 | 123 | 0 | 1081 s, 4324 s |
| Phase 3 | 71 | 68 | 3 (Q14, Q18, Q29) | 619 s, 2476 s; after the pause 606 s, 2424 s |
| Phase 2 | 99 | 95 | 4 (S15, S24, CAP2, CAP3) | 575 s, 2300 s; after the restart 511 s, 2044 s |
| Phase 1 | 45 | 43 | 2 (B3, B4) | 440 s, 1760 s |
| Total | 395 | 386 | 9 | |

- Every outcome is what `mutation/faults.py` expects: the nine survivors
  are those of `970316f`, each documented there as equivalent, not
  observable or unreachable. No fault was invalid.
- Caught at the timeout: MR9, MR25, MR26, CS29, CS30, Q1, T6.
  - CS30, MR25 and MR26 turn a loop into one that never waits; with time
    paused a test waits for good (CS30 as in Phase 4; MR25 and MR26 in
    `docs/DESIGN_QUESTIONS.md` 12.7). CS30 also had its two supervisor
    tests failing.
  - MR9, Q1 and T6 fell in parts of the run that left no result file, so
    their failing tests were not recorded. Each was run again alone on
    `91bf22c` with one worker, on 2026-10-14 from 16:13 to 17:35: MR9
    made `a_failed_attempt_is_not_repeated_at_once` fail, Q1
    `without_a_socks_endpoint_there_is_no_connection_at_all` (as on
    `970316f`), T6 324 named tests (on `970316f`, alone, dozens in 791
    s). Each also reached the timeout of 1200 s; no catch depends on it.
  - CS29 was caught by the timeout alone; on `970316f` four supervisor
    tests caught it in 298 s. The cause is a Phase 5 test helper:
    `Peer::start` in `tests/runtime.rs` waits for its service to be
    published with `until`, which allows 864,000 steps of 100 ms of
    paused time; under CS29 the service is published under a generated
    key, so the wait never succeeds, and at about 16 ms real per step one
    start lasts hours. `tests/runtime.rs` runs before
    `tests/supervisor.rs`, so the supervisor tests were not reached. The
    fault is caught, and the supervisor tests are unchanged: outside the
    runner, in a mirror of `91bf22c` with CS29 applied, `cargo test -p
    monolith-core --test supervisor` fails exactly the four tests that
    caught it on `970316f`
    (`a_deleted_identity_is_taken_down_and_not_published_again`,
    `a_lost_service_is_published_again_under_the_same_name`,
    `closing_the_installation_takes_its_services_down`,
    `the_supervisors_of_two_identities_do_not_touch_each_other`).
    Bounding that wait is follow-up work (item 29).
- The run was interrupted three times on the same commit, each time
  resumed from the faults that had not ended, with the copies made again
  and the unmodified run measured again: at the owner's request on
  2026-10-12 at 21:31 (Phase 5 after 24 faults; resumed at 22:04), at the
  owner's request on 2026-10-13 at 18:48 (Phase 3 after 51 faults;
  resumed at 22:03), and by a restart of Windows for updates on 2026-10-14
  at about 04:20 (Phase 2 after 37 faults; resumed at 08:11). A fault in
  flight at an interruption was run again from the start; no outcome was
  counted twice or lost (each of the 395 ids has exactly one outcome
  line).

### 25. Private Tor E2E

`tests/tor-network/two-node.sh` with `RESTART_CYCLES=15`, Tor 0.4.9.13,
Chutney `ae3a33c`, strace 6.1, in the container: passed in 2085 s, 60
`ok` lines, no failure. The private network bootstrapped in 97 s.

- Steps 1 to 12 (Phases 3 and 4): passed. The 15 restart cycles of Tor B
  each needed one attempt: 3 to 7 s, except cycle 8 (110 s) and cycle 15
  (161 s, with one dial retry of the node); every SOCKS reply was 0x00.
- Steps 13 to 20 (Phase 5), against the twelve points of the brief:

| Point | Step | Behaviour and timing |
| --- | --- | --- |
| 1, 2 Alice queues while Bob is away; Bob comes back | 14 | "first" reported queued; the attempt deferred (`Tor`) |
| 3 to 6 arrives once, Bob stores it, the acknowledgement returns, Alice records it delivered | 14 | queued to delivered 107 s, 1 attempt deferred; B holds it once, A holds it delivered |
| 7 Bob restarted after receiving, before acknowledging | 15 | B with `--ack-delay 3600`, killed with SIGKILL after storing "second"; the retry was acknowledged and not stored again, 10 s across the crash |
| 8 Alice restarted with a queued message | 16 | A killed with SIGKILL with "third" queued; restart to delivered 31 s; earlier deliveries stayed delivered |
| 9 Tor restarted with a queued message | 17 | Tor A stopped, "fourth" queued, 1 attempt deferred for Tor; Tor A started again, service published again, delivered through Tor 11 s after it was queued; also step 9, 15 restarts of Tor B |
| 10 contact blocked with queued work | 18 | "fifth" queued, B blocked, B back for two minutes: no session, no attempt, no delivery |
| 11 identities apart | 19 | a second identity of A delivered over its own session in 4 s while the first had none and kept its queue; B kept the two conversations apart |
| simultaneous reconnect | 19 | both sides sending at once: both delivered in 1 s, each once, one session |
| 12 no direct, DNS, HTTP or UDP fallback | 20 | 15 runtime node runs under strace: loopback and Unix sockets only, no DNS, no UDP socket |

Before it, in the same container, the fail-closed test (T-NET-1) passed
in 25 s. The Dockerfile does not pin Tor; the version above is the one
of this run.

### 26. Independent-review findings and fixes

Recorded in `docs/DESIGN_QUESTIONS.md` sections 12.4 to 12.7, each with
its fix and, where it has one, its regression test and mutation fault.
The faults MR29 to MR57 come from these findings.

- Internal source-first review (12.4): R5-1 to R5-7. Among them, a message
  could be sent before its queueing was durable (R5-1), a failed store
  made a session resend without end (R5-2), a block or deletion did not
  stop a scheduler (R5-3).
- Two independent reviews of the same tree, neither shown the other's
  findings (12.5): R5-8 to R5-20. Among them, a session aborted with its
  service stayed registered (R5-8), a peer that never acknowledges was
  dialed every minute (R5-9), a head deleted while waiting was sent
  (R5-10), a missing or mismatched store was handled silently (R5-11), a
  crash while the store was made blocked the installation (R5-12), an
  identity being deleted could gain messages (R5-15). R5-20 was a
  failure of the tooling, not of the code.
- A review of the fixes (12.6): R5-21 to R5-27, none critical or high;
  R5-28 not changed, with its reason.
- The runner (12.6): R5-29, a fixed bound that an unmodified run could
  exceed; the bound is now measured.
- The final verification of `75ccf7c` (12.7): R5-30, MR28 survived; a
  test was added (`07d6335`). That change was reviewed independently
  before the second freeze, which is recorded here only: no defect in the
  code, six inaccuracies in the test and its texts, all corrected before
  the freeze (the most important: assertions on B's side could not see a
  resend, and were replaced).
- A fact check of this report against the repository, before its commit,
  found statements of it that did not match the code; they were
  corrected here (items 6, 7, 9, 12, 14, 18, 20, 28). Two of them are in
  the documents of the repository too: the wording of item 20 and the
  unused `RefusedReason::Storage` (item 29).
- Findings not changed, with their reasons, are listed in 12.4 (R5-7),
  12.5 and 12.6. No production finding is open.

### 27. Phase 1 to 4 regression status

- Every test of Phases 1 to 4 passes on all three toolchains (item 21).
  No old test was weakened: of the test files that existed at
  `phase-4-complete`, `identities.rs` gained the two new fields of a
  Phase 4 vault (`id: None`, `PayloadVersion::V1`) with no assertion
  changed; `structure.rs` and the two `fuzz_seeds.rs` gained checks and
  seeds, and the seeds of Phase 4 are produced and committed unchanged.
- The mutation lists of Phases 1 to 4 are unchanged: same ids in the
  same order, each fault identical to `phase-4-complete`. On `91bf22c`
  they give the outcomes of `970316f` (item 24); the one difference is
  CS29, caught in the run by the timeout instead of by its named tests,
  for the reason given there, while those four tests still fail under it
  when they run.
- The steps of Phases 3 and 4 of the private network test and the
  fail-closed test pass (item 25).
- The fuzz targets of Phases 1 to 4 run without a crash, and
  `vault_payload` and `contact_store` ran 600 s each.

### 28. Residual documented limitations

- `docs/DESIGN_QUESTIONS.md` 12.3: no "history off" mode or automatic
  retention (whole conversations are deleted); the per-contact rates of
  `RESOURCE_LIMITS.md` section 6 are not enforced; the message store
  rewrites its whole file, so long histories need the user to delete
  conversations; one message in flight per contact; queued messages of
  ended conversations count toward the queue of the identity until
  deleted; an identity whose deletion is cancelled outside the runtime
  keeps its messages until the next opening; Profile messages and file
  transfer are ignored when they arrive.
- Accepted with their reasons in 12.4 to 12.6: the duplicate check is
  per contact record (R5-7); queries read the state in memory, ahead of
  the store by a write under way; blocking or deleting by the remote
  identity of a record, raced on another thread by a deletion and a new
  import of the same identity, acts on the new record; two sessions of
  the same initiator registered in different orders can close each
  other; a session waiting for the store to write its head reads nothing
  meanwhile (R5-28).
- ADR 0008: a rollback of the message store by someone who can write the
  directory is not detected (a delivered message can become queued again,
  and the receiver drops the retry by identifier); the size of the store
  and the times it is written are visible in steps of 64 KiB; deletion
  does not reach older copies or freed blocks.
- The duplicate check covers a window of 1024 deleted identifiers per
  conversation; a retry of a message deleted beyond it would be taken in
  again (ADR 0007).
- Mutation: MR25, MR26 and CS30 are caught by the timeout alone, by the
  nature of the fault; CS29 is caught by the timeout alone because of the
  wait of `Peer::start` (item 24).
- From Phase 4, unchanged: C1, ST6, MI-1, MI-4.
- Rust issue rust-lang/rust#164250 (reported on 2026-10-13, I-unsound, no
  fix yet; stable 1.85.0 and later, so the minimum version too): a
  covariant field assignment can bypass a struct's lifetime bounds. The
  pattern it needs, a type whose `Drop` relies on a `'static` bound of
  its parameter, is absent from Monolith: its only type definitions with
  a `'static` bound on a parameter take `B: TorBackend`, implemented
  only by owned types without lifetimes, and no field that holds a `B`
  is assigned after construction. Its other `'static` bounds are on the
  futures and values of function generics handed to `spawn`, and on the
  associated type `TorBackend::Stream`. `forbid(unsafe_code)` does not
  protect against it; the reason is the absence of the pattern. A text
  scan of the 74 external packages of `Cargo.lock` found types with a
  `'static` bound and a `Drop` only inside tokio, where Monolith cannot
  name or build them, and in `wit-bindgen`, which is not built for
  Linux; it is a scan, not a review, does not see types made by macros
  and does not exclude other variants of the cause. `cargo audit` and
  `cargo deny` do not prove the absence of unknown soundness bugs.
  Building and testing again with the fixed compiler, once released, is
  a check, not a proof.

### 29. Deferred Phase 6 work

- The GUI on the runtime, which the brief of Phase 5 names as Phase 6;
  the order of the later phases is the owner's next brief (12.3).
- In the runtime API: contact requests, invitations and other commands
  the core already has.
- Open points of 12.3: history modes, per-contact rates, a store for long
  histories, several messages in flight, taking in Profile messages and
  file transfer.
- Bounding the wait for publication of `Peer::start` (and `until`) in
  `tests/runtime.rs` to minutes of test time, so that a fault like CS29
  fails by name.
- Building and testing again with the stable compiler that fixes
  rust-lang/rust#164250.
- Documents: the wording of ADR 0007 and of `api::Event` on when events
  are raised (item 20), the card condition of the scheduler in ADR 0009
  (item 14), and the unused `RefusedReason::Storage` (item 9).

### 30. Final verdict

Against the completion rule of the brief:

- durable outbox and inbox, acknowledgement semantics, idempotent
  retries, suppressed duplicates, recovery after restarts: items 5 to 11;
  T-MSG-1 to T-MSG-4 and T-MSG-11; steps 14 to 17 of the network test;
- the session manager converges: items 12 and 13; step 19;
- reconnecting is bounded and goes through Tor only: item 14; steps 17
  and 20; T-NET-1;
- the lifecycles of contacts and identities end stale work: items 15 and
  16; step 18;
- commands and events fit a later interface: items 19 and 20;
- resource use is bounded: item 18;
- no message text in diagnostics: T-MSG-12;
- the invariants and the mutation baseline of Phases 1 to 4 hold: item
  27;
- every Phase 5 fault behaves as expected, 57 of 57 caught: item 24;
- fuzzing and the private Tor network test are green: items 23 and 25;
- no production finding of the reviews is open: item 26;
- the final verification is green on one frozen candidate, `91bf22c`:
  items 2 and 21 to 25.

The limitations of item 28 are documented and none breaks a requirement
of the brief. The work of item 29 is not needed for this verdict.

PHASE 5 COMPLETE
