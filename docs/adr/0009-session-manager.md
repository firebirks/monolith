# ADR 0009: Session manager, duplicate sessions and reconnecting

Status: decided for Phase 5.
Date: 2026-10-11.

## Context

Until Phase 4 whoever held a link decided what to do with it: the
development node kept every session open and sent on the newest. Phase 5
needs one owner for the sessions of each local identity: one that knows
which session carries the queue of a contact, keeps one when both sides
dialed at once, dials when there is something to deliver and only then,
and stops at once when the contact or the identity is gone.

`PROTOCOL.md` section 14 already defines the duplicate rule, and
`monolith-protocol` implements it (`duplicate::resolve`). Section 15
leaves the reconnect timing to local policy; `RESOURCE_LIMITS.md` section
7 gives its delays.

## Decision

### One session manager per local identity instance

The runtime holds, for each open `LocalIdentity`, one session manager.
Its key is the contact record a session stands for, by its `ContactId`
(ADR 0008), not the remote identity: two records of the same remote
identity, one after the other, never share a session or a queue.

- Every link, dialed or answered, runs in one task that owns it. Only
  that task sends and receives on it.
- A session is registered when it reaches `AuthenticatedContact` and the
  contact store says, under the lock of the contact, that it stands for
  the record of that `ContactId` (S50). Before that, and for any other
  standing, it delivers nothing of the queue.
- The manager chooses the preferred session of each contact record. Only
  the preferred session sends queued messages. Any registered session of
  the record takes in messages and acknowledgements.
- A session leaves the manager when its link ends: a Close, a failure, a
  withdrawal by the contact store (block, deletion, a retired key), a
  deadline, or the identity closing.

### Duplicate sessions

When a second session of the same record registers, the manager applies
`PROTOCOL.md` section 14 through `duplicate::resolve`, with the identity
keys of both sides, who initiated each session and whether its transport
key is still the active credential:

- same initiator: the newer stays, the older is closed with Close;
- different initiators: the session initiated by the side with the
  smaller identity key is preferred. If it is the newer, the older is
  closed. If it is the older, it is probed with a Ping with a fresh random
  nonce (a fixed one if the random source of the system fails: one Ping is
  outstanding at most, so its Pong is told by the state of the session,
  not by the nonce); a Pong with that nonce within
  `DUPLICATE_PROBE_TIMEOUT` keeps it and the newer is closed, otherwise
  the older is closed. The timeout counts only while the session is read,
  so a wait for the store does not use it up;
- a session whose key is no longer the active one loses; if neither is,
  both are closed. That is looked at again when a probe ends: a session
  whose key was retired while it was probed loses, answered or not.
  At a registration only the newer session can lose this way: a session
  registered earlier whose key was retired has been withdrawn by the
  contact store, so the runtime gives it as active. The two outcomes of
  `duplicate::resolve` for an older session with a retired key (the
  newer stays; both are closed) are therefore not reached by the runtime;
  `monolith-protocol` tests them.

When both sessions are alive, both sides keep the same one, because the
choice depends only on the two identity keys and on who initiated each
session. While a probe runs, the older session keeps the queue. A message
that was in flight on a closed session stays queued and is sent again on
the one that stays; the receiver drops it by identifier.

This is not the budget for strangers: only sessions that stand for a
contact record take part, and a stranger never reaches the manager.

### Keepalive

Monolith sends no Ping to keep a session alive. A Ping is sent only as
the probe of a duplicate session. A Ping from the peer is answered with a
Pong carrying its nonce. A Pong that answers no outstanding Ping is a
violation and ends the session without a Close. One Ping is outstanding
at most; a probe asked while it is waits for its Pong, and a Pong that
comes after its probe was decided is taken and dropped. A Ping not
answered within `PONG_TIMEOUT` ends the session with Close; like the
probe, that time counts only while the session is read.

A session is kept while it has a reason: a message queued or in flight
for its contact, in either direction. When for `SESSION_LINGER` no
ChatMessage was sent or taken in on it and no MessageAck on it delivered
a message, the local side closes it with Close. A message held already,
which is acknowledged again, and an acknowledgement that delivers
nothing do not count: a contact cannot keep an idle session open with
them (`DESIGN_QUESTIONS.md` P7-6). `IDLE_TIMEOUT` stays the bound of the
session layer.

### Reconnecting

One scheduler per conversation, while all of these hold: the conversation
has queued messages, its `ContactId` is the current record of the remote
identity and that record is a requested or accepted contact, the
identity is open, and no session of the record is registered. A record
without a card confirmed for dialing has a scheduler too: each attempt
finds no card in the dial plan and is reported `DeliveryDeferred` with
`NoEndpoint`, after the same growing delays, and nothing is dialed.

- The first attempt follows the command that queued the first message at
  once; at startup, attempts are spread at random over
  `STARTUP_DIAL_SPREAD`. After a failed attempt, or a session that ended
  with messages still queued, the next follows after the delays of
  `RESOURCE_LIMITS.md` section 7: 10 seconds doubling up to 30 minutes,
  each times a random factor between 0.5 and 1.5. The delay starts over
  once a session of the record stayed registered for
  `RECONNECT_RESET_AFTER`.
- An attempt is `link::dial_plan` over the cards of the dial plan of
  the record (`PROTOCOL.md` sections 4 and 11.4): the authorized
  successor if there is one, then the active key, the second only when
  the first did not answer, that is, its handshake ended before message
  2 authenticated the responder (since Phase 7, P7-1; before, only the
  first card was dialed). Each card is dialed as `link::dial` dials it,
  through the Tor backend, within `MAX_CONCURRENT_DIALS`. A failure of
  Tor, SOCKS or the handshake changes nothing in the queue; the attempt
  is reported as deferred once, with the failure of its last card, and
  tried again later. A deferral is reported only when another attempt
  follows: not when the record ended, the identity closed or a store
  failed meanwhile (since Phase 7, P7-7). There is no other path: no
  direct connection, no name resolution.
- The scheduler checks before every attempt that its `ContactId` is still
  the current record and that the identity is open, and ends otherwise.
  A dial in progress is dropped when a session of the record registers
  (an inbound one), when the identity closes, or at shutdown.
- An attempt that comes back with a session of another record of the same
  remote identity (the contact was deleted and added again during the
  dial) gives that session to the manager under the record it stands
  for; it carries that record's queue, never this one's.
- Nothing is dialed for a contact without queued messages: no presence
  probing, no periodic connection.

### Lifecycle

Blocking or deleting a contact withdraws its sessions (Phase 4); the
manager drops them, the scheduler of the record ends, and its queue stays
stored, ended. The command names the remote identity of the record in
its work, with nothing awaited before the change, so a record ended and
made again before the work runs is not taken for the one it names; a
send looks at its record again once its wait for the vault is over, and
queues nothing for one that ended meanwhile. An identity that closes
ends its supervisor, its sessions and its schedulers; commands for it
fail. Old tasks find their record or identity gone and change nothing.

A session leaves the manager when its task ends, however it ends: the
end is handled when the task is dropped, also when the accept loop of a
service that went away aborts it. A scheduler that ended a session for
want of an acknowledgement keeps its delays growing; only a session that
ended otherwise after `RECONNECT_RESET_AFTER` starts them again. A
failed vault or message store ends every session and scheduler of the
runtime. A runtime whose last handle is dropped without a shutdown stops
as at a shutdown.

## Consequences

- Sessions exist only while there is something to deliver, so an idle
  pair of contacts has no traffic beyond Tor's own.
- A contact that comes online is reached when either side has something
  queued; the side without anything queued waits to be dialed.
- A probe costs one Ping and up to `DUPLICATE_PROBE_TIMEOUT` before the
  newer of two live sessions is closed.

## Alternatives considered

- Keep sessions open with periodic Pings: presence information on the
  wire and traffic without a reason.
- Choose a duplicate by a random token exchanged on the session: needs a
  wire change, and the existing rule converges already.
- A scheduler per remote identity: would let a queue pass to a contact
  made again for the same identity.
