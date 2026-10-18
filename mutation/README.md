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

- `faults.py`: the lists `PHASE1` to `PHASE6`. An entry names the
  file, the exact text to replace and its replacement, and what the fault
  means. `expect` is `"caught"` or the reason why the fault survives.
- `run.py`: the runner.

## Running

    python3 mutation/run.py all 3
    python3 mutation/run.py phase6 3
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
with `--no-fail-fast`, so the result names every test that noticed. When
`monolith-ui` is among them, its `fixtures` feature is on, so that the
tests of the screens run in the states their fixtures build. The runner
runs on Linux.

Outcomes:

- caught: a test failed, or the run hit the timeout;
- survived: every test passed;
- invalid: the fault does not compile, which is a mistake in the list.

The timeout is measured, not fixed: it is the slowest unmodified run of
a copy, twice over for each worker, and never less than 20 minutes. The
unmodified run itself has two hours. With Phase 5 the unmodified run
takes about 16 minutes on four cores, so a fixed bound of 20 minutes no
longer told a fault that hangs from tests that are slow because other
copies run beside them: a fault that survived could have been counted
caught. The log gives the unmodified run and the timeout, and marks
each fault that reached it.

The results go to `mutation/results-<list>-<commit>.json`. The exit code
is 0 if every fault had the outcome its entry expects, and 1 otherwise. A
full run of all six lists takes more than a day with two workers on
four cores; a fault caught only by the timeout takes the whole of it.

When code moves, a pattern may no longer match. The runner then stops
before testing and names the entry; update the pattern so that the fault
stays the same fault.

## Expected results

With Phase 6; the final verification on `68ac655` gave exactly this,
every fault with the outcome its entry expects:

| List | Faults | Caught | Expected to survive |
| --- | --- | --- | --- |
| Phase 1 | 45 | 43 | B3, B4 |
| Phase 2 | 99 | 95 | S15, S24, CAP2, CAP3 |
| Phase 3 | 71 | 68 | Q14, Q18, Q29 |
| Phase 4 | 123 | 123 | none |
| Phase 5 | 57 | 57 | none |
| Phase 6 | 160 | 160 | none |

555 faults, 546 to be caught, 9 expected to survive. The first five
lists gave the same on `91bf22c`, the verification of Phase 5, and the
first four on `970316f`, that of Phase 4.

Phase 4 was closed on 2026-10-11, Phase 5 on 2026-10-14 and Phase 6 on
2026-10-18. From then on the faults and the regression tests of Phases
1 to 5, and UI1 to UI160, are a baseline to preserve: none is weakened
without an explicit design decision.

On `91bf22c` CS29 was caught by the timeout alone: under it the wait for
publication of `Peer::start` in `tests/runtime.rs` lasts hours, so its
four supervisor tests were not reached (they fail when run on their
own). On `68ac655` it still reached the timeout, but twelve tests of
`tests/front_end.rs`, which runs before `tests/runtime.rs`, failed
first. Bounding that wait, so that CS29 is killed quickly, is left for
later (`docs/DESIGN_QUESTIONS.md` 12.8 and 13.3).

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
  waiting (MR26), and a message in flight sent again on the same session,
  its acknowledgement then awaited from the last send (MR28);
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

MR25 and MR26 are caught by the timeout of the runner alone, MR9 by the
timeout and a test that fails: each turns a loop of the runtime into one
that never waits, so with time paused the tests that wait for a session
to end wait for ever, as with CS30. MR28 survived the final verification
of `75ccf7c`: no test queued more while an acknowledgement stayed away
and looked at when the session ended. It is caught since by a test
written for it
(`docs/DESIGN_QUESTIONS.md` section 12.7).

The Phase 6 list, UI1 to UI160, covers the desktop client
(`docs/adr/0010-gui-toolkit-egui.md` to `0012-design-system.md`) and
what its reviews added:

- the UI model: a late answer or event of one identity taken for another,
  contacts of another identity kept, a confirmation that acts on the
  selection instead of the handles it captured, missed events not followed
  by a full read, drafts cleared by a refused send or by a send of another
  conversation, the state of a deleted identity kept, a dialog of one
  identity left open over another, a burst of events read one by one, an
  ended conversation or a request no longer listed acted on, an answer to
  no command in flight taken, and a failure told in its debug form (UI1 to
  UI14);
- secrets and diagnostics: the passphrase in `Debug` output, the card of a
  closed dialog kept, the undo history of a secret field or of the
  composer, the input method left on in a secret field (UI15, UI27, UI31,
  UI47, UI48);
- the boundary of the window: a request to open a URL that reaches the
  system, sessions coming and going that reach the window (UI16, UI21);
- the bridge: a request or invitation number taken for another identity,
  the references of one identity forgotten with another's, a number given
  twice, the answer of one command given for another (UI17 to UI20);
- files: a settings value that adds a line, Tor settings without a cookie
  file, a saved card that replaces a file, the size limit of a card file,
  a device read as a card (UI22 to UI26);
- the screens and the design system: a queued message marked delivered,
  a stock egui label, a sender's chosen name made the title of a request,
  a colour of a screen's own, a disabled or busy button that takes clicks,
  the keyboard reaching behind a modal, a modal closed while its action
  runs, a control that takes no input reacting to the pointer (UI3, UI28
  to UI30, UI32 to UI34, UI49);
- how a peer's text is drawn: a direction-changing character drawn as it
  is, an over-long grapheme cluster drawn whole, a collapsed message drawn
  in full (UI44 to UI46);
- the proof that the SOCKS endpoint is Tor's (`docs/THREAT_MODEL.md`
  adversary O): a dial without it, a proof that outlives its lifetime,
  any listed listener taken for the configured one, a failed dial or a
  lost service that leaves the proof standing, waiting dials that each
  make their own, a filter's answer taken as a proof, a status query that
  greets an unlisted endpoint, and the desktop that dials without the
  proof (UI35 to UI43);
- a card is not an invitation (`docs/DESIGN_QUESTIONS.md` P6-20): every
  contact or every added card said to carry an invitation, by the runtime
  or the bridge, "Add contact" closed after a plain card or kept open after
  the card of an accepted contact, the card typed kept once the contact is
  added, a plain card's contact said to wait for an acceptance, the
  waiting notice's button or "Show my card" that opens nothing, the own
  card that ignores the request mode, and a card file read that lands in a
  dialog whose contact was added (UI50 to UI61);
- closing and delivery: what was typed during a send cleared, or the sent
  text left in the draft, ended conversations counted as waiting, a session
  that saves nothing or messages not delivered that close without a
  question, a close agreed to that asks again, a close not cancelled, the
  window hidden while it asks, Close Monolith that does not close, the
  question not asked over an open dialog, shortcuts behind it, and the time
  of the next attempt counted from nothing, kept with nothing queued, said
  while Tor is down or never said again (UI62 to UI76);
- unblocking (P6-21): a handle of another identity or of an instance from
  before, the first blocked identity unblocked whatever the handle, every
  record listed as blocked, someone not listed unblocked, the blocked people
  of another identity kept, a late answer that reads the identity shown, a
  refused unblock told as done, a number kept for someone unblocked, a
  refusal told as unexpected, and the Unblock button of one row that acts
  on another or does nothing (UI77 to UI87);
- what a frame draws: a preview that reads a whole long message or loses
  its ellipsis, prepared texts kept for a message that changed or not made
  again when it is expanded, the state of a conversation whose messages
  were deleted kept by the window, the rows of the list not made again or
  keeping an old preview or unread count, the reader moved by older
  messages, a conversation that does not open at its newest message, every
  message laid out, a misplaced frame shown, a reader taken back to the
  bottom, initials for contacts without a name (UI88 to UI103);
- the window around the screens: shortcuts behind a question, Tor not
  checked again after missed events, a runtime thread that ends without
  waking the window, threads waited for while the window is still up, a
  card file swapped or linked in place of the one looked at (UI104 to
  UI110);
- what the screens say and ask: a draft cut by a queued send, an added
  card said as given rather than as kept, declining and unblocking without
  a question, a failed store said again, no Revoke after accepting through
  an invitation, a message being queued not counted before closing, a
  passphrase buffer that moves, a waiting contact said to wait for the
  wrong thing, invitations said to bring requests under Nobody, field
  errors not on their field (UI111 to UI123);
- the look and the keyboard of the design system: fingerprints in rows of
  six, initials on request cards and in the contact dialog, ended
  conversations marked as blocked, markers without words, wells without an
  edge, a narrow settings page, solid red triggers, messages cut against the
  bar, the drop zone, history kept by fields that must keep none, scroll
  bars, the sidebar edge and the composer in the Tab order, a sidebar edge
  or a bar that cannot be dragged, toasts over notices and dialogs, the
  blocked people out of view, the first screens without their mark, the
  switcher's card entry, the unblock question, rows brought into view, the
  requests entry, short first-start windows, the place of Close in the Tor
  details, card text that does not fill its lines, and boxes wider than
  their column (UI124 to UI160).

UI33 first removed the scrim's interaction and the modal layer together;
the scrim's own widget still shielded the pointer, so it survived. It is
now the modal layer alone, which lets the keyboard reach what is behind a
modal, and a test catches it.

A retry that goes around Tor has no fault: no crate outside `monolith-tor`
opens a socket (`docs/SECURITY_INVARIANTS.md` S1), and every attempt of
the runtime goes through `TorBackend::connect_onion`; a path around it
would be new code, not a changed check. The bound on the length of a
message is that of `ChatText`, which X1 and X2 of Phase 1 keep; the
runtime checks the text with it before anything is stored.

The Phase 7 list, HR1 and up (H is taken by the handshake faults of
Phase 2), covers the hardening of `docs/DESIGN_QUESTIONS.md` section
14:

- the dial plan during a rotation of the peer's key (P7-1): only the
  first card dialed, the next card dialed after a card that answered or
  after a failure of Tor, or after the attempt is over (HR1 to HR4);
- the words of a sender and names in sentences of the client (P7-2): the
  name a sender chose put in the caption of a request or in the sentence
  of the Accept dialog, a name drawn as the client's words, a name of the
  model made words of the client (HR5 to HR8);
- files of this computer (P7-3): an offline or cloud-only file held on
  the disk, a device name with an extension, a path resolved on a share
  taken as one on a drive (HR9 to HR11); and, in code built on Windows
  only and run there as the list `phase7w`: a link on the way followed, a
  card file not compared with the one looked at, a device name taken as a
  file, a cookie file or a data directory on another computer taken
  (HR12 to HR16);
- file messages on a runtime that transfers no file (P7-4): an offer
  ignored, a file message for a transfer never offered ignored, a
  rejected offer not remembered, the ended transfers kept without bound
  (HR17 to HR20);
- a store above a limit a later build lowered (P7-5): a queue or the
  store that ignores the lowered limit, a lowered window of forgotten
  messages not applied (HR21 to HR23).

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
