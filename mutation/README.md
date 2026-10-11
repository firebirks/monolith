# Mutation testing

A list of single faults in the checks that matter, and a runner that
applies them one at a time and runs the tests. Every fault has to make a
test fail, unless its entry says why it cannot. A fault in authentication
logic that no test notices blocks a phase (`docs/TEST_PLAN.md` section
14).

This is not an automatic mutation tool. Each fault is chosen for a rule of
the specification: a comparison removed, a bound moved, a check skipped, a
key kept.

## Files

- `faults.py`: the lists `PHASE1` to `PHASE5`. An entry names the
  file, the exact text to replace and its replacement, and what the fault
  means. `expect` is `"caught"` or the reason why the fault survives.
- `run.py`: the runner.

## Running

    python3 mutation/run.py all 3
    python3 mutation/run.py phase5 3
    python3 mutation/run.py phase4 3
    python3 mutation/run.py phase3 3
    python3 mutation/run.py phase2 3
    python3 mutation/run.py phase1 3
    python3 mutation/run.py all 3 H7 S1 S2

The first argument selects the list, the second the number of workers,
and any further arguments restrict the run to those faults. The runner
needs `git`, `cargo` and Python 3, and nothing else.

It works on copies of the committed `HEAD` under `mutation/work/`, one per
worker, each with its own target directory, so the working tree is never
touched and uncommitted changes are not tested. Before anything runs it
checks that every pattern occurs exactly once in its file, and that the
unmodified copies pass their tests. Then it checks that each copy builds
what it holds, on its own target directory: the unit tests of the
crates of the unmodified run pass, fail with fault X1 applied, and pass
again once the file is restored, with `monolith-protocol` built again. A fault is applied only
if the file then holds it, and every file is restored to its exact bytes
with a time newer than any build, so that cargo never runs the build of a
fault after it. A copy that fails one of these checks stops the run: the
build system, not the code, would decide the results after it. For each fault it runs the tests of
the crate that holds the fault and of every crate that depends on it,
with `--no-fail-fast`, so the result names every test that noticed.

Outcomes:

- caught: a test failed, or the run hit the timeout;
- survived: every test passed;
- invalid: the fault does not compile, which is a mistake in the list.

The results go to `mutation/results-<list>-<commit>.json`. The exit code
is 0 if every fault had the outcome its entry expects, and 1 otherwise. A
full run of all five lists takes several hours with three workers; a
fault caught only by the timeout takes the whole of it (20 minutes).

When code moves, a pattern may no longer match. The runner then stops
before testing and names the entry; update the pattern so that the fault
stays the same fault.

## Expected results

With Phase 4; the final verification on `970316f` gave exactly this,
with exit code 0 for each list:

| List | Faults | Caught | Expected to survive |
| --- | --- | --- | --- |
| Phase 1 | 45 | 43 | B3, B4 |
| Phase 2 | 99 | 95 | S15, S24, CAP2, CAP3 |
| Phase 3 | 71 | 68 | Q14, Q18, Q29 |
| Phase 4 | 123 | 123 | none |

338 faults, 329 to be caught, 9 expected to survive.

Phase 4 was closed on 2026-10-11. From then on the faults CS1 to CS123
and the regression tests of Phase 4 are a baseline to preserve.

Phase 5 adds the list MR1 to MR57, all expected to be caught; the table
gets its row from the run of the final verification of Phase 5.

The Phase 5 list, MR1 to MR57, covers the messaging runtime
(`docs/adr/0007-delivery-semantics.md` to `0009-session-manager.md`):

- acknowledgements: a message acknowledged before it is durable (MR1),
  recorded delivered when it is queued, when it is handed to a session or
  reported so when its session ends (MR2, MR29, MR30), an
  acknowledgement that delivers a message of another conversation or of
  another local identity (MR4, MR5), or counted on a session that no
  longer stands (MR17);
- deduplication: a message received again stored again, a deleted one
  taken in again (MR3, MR20);
- durability: queued messages not written, a delivery written as queued,
  a message reported queued before it is durable, a header of the message
  store that is not authenticated (MR6, MR15, MR16, MR18);
- lifecycle: a contact made again with the identifier of its earlier
  record, a scheduler that goes on after its record ended, the messages of
  a deleted identity kept, a conversation of a record of another remote
  identity accepted, a vault of version 1 not written with its new
  identifiers, an identity restored from its keys with the identifier of
  the deleted one (MR7, MR8, MR21 to MR24);
- sessions: the duplicate rule of `PROTOCOL.md` section 14 broken three
  ways (MR10, MR11, MR27), an idle session that never ends (MR25), a
  message whose acknowledgement does not come that keeps its session
  waiting (MR26), and a second message sent before the first is
  acknowledged (MR28);
- reconnecting and limits: attempts without a delay (MR9), a conversation,
  the queue of a contact or of an identity one past its bound (MR12 to
  MR14);
- diagnostics: message text in `Debug` output (MR19);
- the fixes of the reviews of the Phase 5 code (`docs/DESIGN_QUESTIONS.md`
  sections 12.4 and 12.5): a message offered or sent before it is durable
  (MR31, MR32), a session or a scheduler that goes on after a store failed
  (MR33, MR34, MR44), a dial or a scheduler a block or a deletion does not
  stop (MR35, MR36), a deleted message sent after the wait for the store
  (MR37), a session task aborted with its service that stays registered
  (MR38), delays that start again after a session that ended for want of
  an acknowledgement (MR39), a missing store made again without saying so,
  and a vault of Phase 4 taken beside a store with messages (MR40, MR41),
  what an interrupted creation of the store left that stops it from being
  made (MR42), a runtime dropped without a shutdown that runs on (MR43),
  more identities or sequences than a payload holds (MR45, MR46), a change
  of nothing that wakes every session (MR47), a message of an identity
  being deleted added after its messages were removed (MR48), a send
  left half done when its caller stops waiting (MR49), and a wait for
  durability that marks the outcome seen only after it looked at the
  state, and sleeps through an outcome published in between (MR50, in the
  shared `persist.rs` of Phase 4);
- the fixes of the review of those fixes (`docs/DESIGN_QUESTIONS.md`
  section 12.6): a failed vault refused as a closed identity, a deletion
  reported as a failed store, a failure reported only by what runs into it
  (MR51 to MR53), delays started again after an inbound session that timed
  out (MR54), a command a shutdown does not wait for (MR55), availability
  reported again without a change (MR56), and the end of an unregistered
  session that wakes every scheduler (MR57).

A retry that goes around Tor has no fault: no crate outside `monolith-tor`
opens a socket (`docs/SECURITY_INVARIANTS.md` S1), and every attempt of
the runtime goes through `TorBackend::connect_onion`; a path around it
would be new code, not a changed check. The bound on the length of a
message is that of `ChatText`, which X1 and X2 of Phase 1 keep; the
runtime checks the text with it before anything is stored.

The Phase 4 list, CS1 to CS123, covers the contact store and the vault:

- one decision about a card and no rollback: an older card of the active
  key that rolls the contact back, a stale card promoted, a conflicting
  card taken, the retired key accepted again, an import that judges a
  card apart from an admission, a newer key without a successor called
  stale, an import that makes a blocked identity a contact (CS1 to CS7);
- the store: a block or a deletion that leaves sessions standing, a
  session that no longer stands forgotten without being withdrawn, an
  operation on a copy of the record outside its lock, a confirmation
  without a contact, an import without a generation or returned before it
  is durable (CS8 to CS14);
- message 3: written before the admission is durable, without looking at
  the withdrawal, or before a contact slot is taken, and a contact
  session without a slot (CS15 to CS17, CS44);
- invitations and requests: the bound of the active set, a capability
  outside it, the quota per capability, revoking and discarding (CS18 to
  CS21);
- several identities: an isolation group shared between contacts, two
  identities with one key (CS22, CS23);
- strangers: the newest evicted instead of the oldest, a stranger that
  has spoken evicted, draining links without a bound, one slot more, an
  evicted stranger's message delivered (CS24 to CS28);
- the supervisor and rotation: a new key on publishing again, attempts
  without a delay, a switch or an end of rotation that is due at once,
  the pending successor dialed, a confirmed card dialed whatever key it
  states (CS29 to CS34);
- the vault: a write in place, a directory not flushed, a header not
  authenticated, KDF parameters outside the bounds, a passphrase not
  normalized, a directory others can reach, no single-instance lock,
  padding that is not zero, a contact stored twice (CS35 to CS43);
- the fixes of the review of the Phase 4 code (`docs/DESIGN_QUESTIONS.md`
  section 11.4): a write whose waiter is cancelled loses the vault
  (CS45), a deleted identity that is not closed, changes it in memory, or
  is published again (CS46 to CS48), a successor announced or a new key
  used before the step is durable (CS49, CS50), a snapshot without the
  lock of the map (CS51), an admission whose cancelled wait is never
  forgotten (CS53), a rotation ended before its switch is durable (CS54),
  and a failed write that withdraws nothing when no waiter is left
  (CS55). CS52, a decline that lets go of the map between its two slots,
  is caught by a test that takes a snapshot at a hook between the two
  slots; CS51 by one that looks, at a hook after every slot a snapshot
  reads, whether the snapshot still holds the lock of the map, whatever
  the order in which the map yields the slots;
- the closure of the review of the Phase 4 code
  (`docs/DESIGN_QUESTIONS.md` section 11.5): stored secrets in `Debug`
  output (CS56), keys the vault would refuse or a rotation to the same
  public key (CS57 to CS59), a deleted identity that signs, hands out a
  party, keeps its secret bytes, or takes what a late session or a
  waiting dial brings (CS60 to CS62, CS81, CS82, CS90), a session of one
  identity applied at another (CS63), what is shown or admits before it
  is durable: an invitation, its card, the successor card, a new identity
  (CS64, CS65, CS84 to CS86), the order of a request and the record of
  its sender (CS66, CS67), what a refused or slot-less admission recorded
  and the writes of a job (CS68 to CS72, CS88, CS89, CS92 to CS94), the
  withdrawal looked at before every write, a link whose write or receive
  was dropped (CS73, CS74, CS83, CS91), a publication when its identity
  is deleted (CS75, CS76), and the progress of one rotation counted for
  another (CS77 to CS80, CS87);
- a logical name taken for an object instance, and what the reviews of
  those fixes found (`docs/DESIGN_QUESTIONS.md` section 11.5, the last
  two tables): the identities of a closed installation that keep their
  authority (CS95), a handle of an invitation or a request that names no
  instance or no request (CS96, CS99), an announcement or a confirmation
  counted for a contact made again or on a session that no longer
  stands (CS97, CS98, CS110), a deletion that takes the identity of the
  same keys (CS100), a switch or an end decided for one rotation that
  changes another (CS101, CS102), an import or a block that takes out a
  request queued after it (CS103, CS104), a card for a capability
  revoked as it is made (CS105), a closed identity that shows a card it
  no longer answered with (CS106), the answering party outside the core
  (CS107), a card dialed of someone who is not a contact or that the
  dial plan does not name (CS108, CS111, CS112), a session that reports
  no capability for its request (CS109), what a link received made or
  changed by a caller (CS113), a wait that reports a failure for a
  generation that is durable (CS114, CS115), an acceptance or a new card
  that counts for a contact made again (CS116, CS117), what arrived
  applied twice (CS118), an announcement sent on another session or
  counted without being sent (CS119, CS120), a snapshot that lets go
  of the keys between its reads (CS121), what arrived on a link that
  ended counted until its record forgets the link (CS122), and a write
  started for what the job before has written and is about to publish
  (CS123). CS81 removes both checks of the
  closing on the path of a late session; the one in `apply` is a second
  line behind the one in `consider` (CS90) and is not shown on its
  own.

CS28 is caught by a test that evicts the stranger at a hook between
taking its first message and delivering it. CS30 is caught by the
timeout of the runner, with two tests of the supervisor that fail: with
no delay between attempts its loop does not end. CS17 lets a dial go on
without a contact slot, and the tests that expect `LinkError::Budget`
fail at once.

Phase 4 moved P1 to P3, H15, H16, Q25, Q26, Q30, CR1, CR2, CR9, CR18,
CR19, CR24 and CR35 to the code they test now, and removed CR36, CR37 and
CR38 from the Phase 3 list, which would be equivalent now: the admission
a link returns and the session it admits come from one decision of the
store, so reading one or the other is the same (CR36, CR37), and the
store keeps the withdrawal of a link only once the admission succeeded,
so ending it after a failed admission is a second line (CR38).

The Phase 3 list covers the no-clearnet and Tor boundaries (onion address
checks, SOCKS only, the onion name only, loopback endpoints and listener,
isolation), control authentication and the parser bounds, the publication
rules (no `Detach`, non-anonymous mode, key checks, ownership by the
control connection, `DEL_ONION`), the budgets and deadlines of the core,
and the checks the two Phase 3 reviews added (Q27 to Q32). For a fault in
`monolith-identity` or `monolith-protocol` the tests of `monolith-tor` and
`monolith-core` run as well.

The credential binding review added CR1 to CR21, and K2 to K4 and H16 in
the Phase 2 list. They cover the local party that only its own identity
key can make (K2 to K4), outbound admission against the state of the
moment and not of the dial (H16), a newer key without continuity, an
announcement through another key, the promotion and the retired key, the
pending card the user confirms or imported, a second statement at the
successor's epoch (CR1 to CR9, CR20, CR21), the withdrawal of a session
and a link and its end (CR10, CR12 to CR17, CR19), message 3 to a key
the contact has left (CR18), and the duplicate rule with credentials
(CR11). None is expected to survive. A doctest catches K4, so the runner
names no failing test for it.

The integration hardening added CR22 to CR38 and moved CR12, CR16, CR18
and CR23 to the code they test now; Q26 follows the code. They cover:

- message 3 only to a peer that may learn the local identity, read from
  the session and not from the admission returned beside it, with the
  withdrawal looked at before and during its write (CR18, CR22, CR36);
- the identity predicate on the proven key and not on the stale-card
  standing, an older card of the active key, and a proven key older
  than the announced successor (CR23, CR24, CR35);
- the deadlines of a session: an absolute frame deadline, the idle limit
  that bytes of a frame do not move, `UNKNOWN_FIRST_MESSAGE_TIMEOUT`,
  `UNKNOWN_SESSION_TIMEOUT`, the age limit without a complete frame, and
  a Close the session decided (CR25 to CR28, CR31, CR32);
- the one end of a link, which gives back its slot, after a deadline, a
  read error or a message that ends the session, refuses later calls,
  ends a pending send on a withdrawal, and reports a failed admission
  (CR16, CR29, CR30, CR33, CR34, CR38), and the slot of a stranger read
  from the session (CR37).

None is expected to survive. A slot for an outbound session whose
standing fell has no code to break: such a session is not made, which
CR18 tests. A stranger answered without a slot is Q30. A fuzz target that
compares accepted handshake messages with its transcript is not a fault
for this runner, which runs the tests; the seeds of a third party and of
another ephemeral key make such a target fail on its own corpus.

The final hardening added CR39, a link that ended and waits for its
stream again, and CR40, a dial refused before message 3 that loses its
admission, and moved CR12 and CR13 to the code they test now. Neither is
expected to survive. A send on a link that is over without the first
check is equivalent: the session refuses it and the link fails the same
way, so no fault is listed for it. The fuzz targets `handshake_initiator`
and `handshake_responder` assert again that a genuine message with one
byte changed is refused; their seeds hold such messages next to valid
ones of another ephemeral key. That assertion is not a fault for this
runner either.

No fault in authentication logic survives. The Phase 2 list covers each
check that the two reviews of Phase 2 added: the grace after the age
limit (S1, S2), the refusal in `set_transfer_active` (S7), `accept_sent`
and `request_sent` in `may_send` (S13, S14), the invitation comparison
(S16), where the invitation comes from (H13, H14), each call of `end()`
that drops the keys (S20 to S25), the all-zero secret key (K1) and
`OutboundPeer::admit` with its record (H15, H16).

Why these survive:

- B3, equivalent. The card inside a ContactRequest or EndpointUpdate is
  taken with the length of a card without capability, 171 bytes. A card
  whose flag announces a capability needs 187 and fails to decode before
  the check that refuses a capability is reached. The check stays as a
  second line.
- B4, equivalent. Without the early check of the endpoint count the same
  input is still refused, by `take()` or by `ContactCard::decode`.
  `take()` returns a slice and allocates nothing. Only the local error
  category can differ.
- S15, equivalent. A failed session has its logic in `Closed`, and one
  that made its Close is in `Closing`. The logic allows nothing there that
  `send()` could produce, so the two flags in `may_send` are a second line.
- S24, unreachable. The `Internal` path of `seal` is taken only if the
  Noise library refuses a length the frame encoder produces, which no test
  can arrange. `docs/CRYPTOGRAPHY.md` lists it.
- CAP2, not observable. The memory of a dropped value cannot be read
  without unsafe code, which the workspace forbids. The test checks that
  the type promises `ZeroizeOnDrop` and that `zeroize` clears it.
- CAP3, equivalent in result. A constant-time comparison is a timing
  property, which no test measures.
- Q14, equivalent. Tor sends nothing unasked; bytes that follow a reply
  stay in the socket and make the next reply fail, so the connection
  fails either way, one command later.
- Q18, equivalent. 88 characters ending in `==` always decode to 64
  bytes, so the length check of a returned key cannot fire.
- Q29, unreachable. `Endpoint` is opaque and only its validating
  constructors make one; the second check in the connection code cannot be
  reached from a test.
A survivor that is not in this list is a gap in the tests, or a check that
does nothing. Either is fixed before a phase is called done; F9 was such a
gap at the end of Phase 2, and in Phase 3 a handle that ignored control
loss before `accept`, a handshake without its deadline, and the dial and
listener-error paths of the core were; all have tests since. In Phase 4
the targeted run found three: a confirmation for an identity that is not
a contact (CS12), a dial without a contact slot (CS17), and a stranger
answered without a slot while evicted ones drain (Q30); all have tests
since. Some fixes of the reviews have no fault: the flush of the parent
of a new directory, which only a power loss would show; the erasure of
an outgrown encoder buffer and of the card, frame and passphrase buffers
of the review closure, which no test can read without `unsafe`; and the
handling of an admission that fails after it changed a record, which no
path reaches today.

## Adding a fault

Add an entry to the list of the phase that introduces the check. Keep the
replacement as small as the fault: one comparison, one bound, one call. If
the fault cannot be caught, say why in `expect`, and add the reason to the
list above.
