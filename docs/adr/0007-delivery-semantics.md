# ADR 0007: Delivery semantics

Status: decided for Phase 5.
Date: 2026-10-11.

## Context

Phase 4 left a contact store, sessions over Tor and the messages
ChatMessage and MessageAck of `PROTOCOL.md` sections 8.5 and 8.6, but
nothing that queues a message, sends it, takes it in or acknowledges it.
Phase 5 makes text messaging reliable: a message the user sent must not
be lost because Tor was down, the peer offline, the process restarted, a
session replaced, an acknowledgement lost or a stream broken at any byte,
and a message must not show up twice at the receiver.

The wire protocol already has what this needs. A ChatMessage carries a
16-byte identifier from the sender's random source and the text; a
MessageAck carries the identifier back. Both are legal only in
`AuthenticatedContact`, inside the encrypted and authenticated session.

## Decision

Durable at-least-once transport, and exactly-once local acceptance
through deduplication.

### Sender

- A message is accepted from the user only as text that passes the rules
  of `ChatText` (1 to `MAX_CHAT_TEXT_LEN` bytes, `PROTOCOL.md` section 9),
  checked before anything is stored, and only for a contact the local
  identity holds as requested or accepted.
- Its identifier is 16 bytes from the random source of the operating
  system, drawn when it is queued. It never changes and is never reused.
- It is stored as `Queued` in the message store (ADR 0008), bound to the
  local identity and to the contact record it was sent to, before the
  command returns. A successful command means "durably queued", not
  "received". Without a session, or with Tor down, the command succeeds
  all the same, unless a limit refuses it.
- `Queued` means: no authenticated acknowledgement for this identifier
  from this contact is recorded. Whatever was written to a stream before,
  a restart treats the message as still to deliver.
- `Delivered` is recorded only for a MessageAck that arrived on a session
  that stands for that same contact record, for an identifier queued for
  that record. It is made durable before anything reports it. A completed
  write of the frame, a written Noise message or an established session
  is not delivery. `Delivered` never goes back to `Queued`.
- A transient "in flight" mark exists only in the task of a session. It is
  not stored and is not evidence of anything.

### Receiver

    receive authenticated ChatMessage on a session of the contact
    -> check that the session still stands for the contact record
    -> store the message and its identifier in one step (ADR 0008)
    -> wait until that step is durable
    -> send MessageAck

- The identifier is looked up in the same locked step that inserts the
  message. If it is held already for that conversation, nothing is
  inserted, no event is raised, and the MessageAck is sent again once the
  state that holds it is durable.
- If the message cannot be stored (a limit, a failed write), no MessageAck
  is sent. The sender keeps it queued and tries again.
- A MessageAck means: this identifier is durably held in the receiver's
  message store for this conversation. It says nothing about display.

### Acknowledgement

- A MessageAck is applied only to the outbound messages of the
  conversation of the contact record the session stands for. It cannot
  reach another contact, another record of the same remote identity,
  another local identity or a message never queued: an unknown
  identifier is ignored (`PROTOCOL.md` section 8.6). A second MessageAck
  for a delivered message changes nothing.
- No signature is added inside the session: the session is the
  authentication of the peer (`PROTOCOL.md` section 6.1), and the store
  binds what arrived to the contact record of that session.

### Ordering

One queue per conversation, first in first out, and one message in
flight per conversation. The head is sent, and the next one only after
the head is recorded `Delivered` and that record is durable. A message
sent again after a lost acknowledgement is the head again, so retries
never reorder later messages. The receiver keeps what it accepts in the
order it accepted it, which is the sender's order. Throughput is one
message per round trip; correctness is the priority in version 1.

### Retry

- A message is sent again, from the head, on every new session of its
  contact that becomes the preferred one (ADR 0009), and after
  `MESSAGE_ACK_TIMEOUT` without an acknowledgement the session is closed
  and the conversation is scheduled again with the reconnect delays of
  `RESOURCE_LIMITS.md` section 7. Every attempt goes through the Tor
  backend; nothing else is tried.
- Duplicates at the receiver are dropped by identifier and acknowledged
  again.

### Crash boundaries

| Crash | After restart |
| --- | --- |
| sender, before the message is durable | the command did not succeed; nothing to resend |
| sender, after it is durable and before or during sending | `Queued`; sent again |
| receiver, after the frame arrived and before the store is durable | no MessageAck was sent; the sender sends again |
| receiver, after the store is durable and before the MessageAck | the sender sends again; the receiver finds the identifier, stores nothing, acknowledges again |
| sender, after the MessageAck arrived and before `Delivered` is durable | `Queued`; sent again, acknowledged again, then `Delivered` |
| sender, after `Delivered` is durable | `Delivered` for good |

### Events

Events are notifications raised after the state they report is durable.
They are not the state: a consumer that misses some reads the current
state with queries. No correctness property depends on an event being
seen.

## Consequences

- A receiver that cannot store refuses delivery instead of losing a
  message; the sender holds it, within its limits, and backs off.
- One message in flight per conversation bounds what a retry can resend
  and keeps ordering simple, at the cost of throughput on slow circuits.
- The duplicate check covers what the conversation holds and a bounded
  window of identifiers of messages the user deleted locally (ADR 0008);
  a retry of a message deleted beyond that window would be accepted again.
- `PROTOCOL.md` sections 8.5 and 8.6 are amended: the acknowledgement
  follows durable storage, and the duplicate window is kept in the
  message store, not in memory.

## Alternatives considered

- At-most-once (send and forget): loses messages on any failure.
- A sliding window of several messages in flight: more throughput, more
  states to get right across session replacement. Not needed for text in
  version 1.
- Signed acknowledgements: add nothing inside an authenticated session.
- Acknowledge on receipt and store afterwards: a crash in between loses a
  message the sender believes delivered. Ruled out by the brief.
