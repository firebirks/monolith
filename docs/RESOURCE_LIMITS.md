# Resource limits

Every limit below is a constant in `crates/monolith-protocol/src/limits.rs`.
That module is the only place where such numbers are defined. This document
explains them; the two are changed together.

Rule for all sizes: compare the declared size with the limit first, allocate
second. No allocation is sized by peer input that has not passed that
comparison.

The values are starting points chosen by analysis, not measurement. Later
benchmarks may move them. Lowering a limit is a compatible change only for
limits marked "local"; the others are part of the wire protocol. The cost
of the handshake in section 11.1 is measured.

Of the local limits, those of section 5 that bound what the vault and the
message store hold (identities, contacts, the blocked and the declined,
invitations, conversations, queues, forgotten identifiers, messages) are
also counts of their formats. A store is read and written up to the
ceilings of its format, which are these values as the format was made
with; the limits of the build bound only what is added. A later build may
lower one of them: a store above it is still read and kept, and nothing
more is added past the limit until it is below it again. Raising one
above its ceiling needs a new version of the format; the build refuses to
compile otherwise (`DESIGN_QUESTIONS.md` P7-5).

## 1. Wire sizes (protocol)

| Constant | Value | Notes |
| --- | --- | --- |
| `HANDSHAKE_PROLOGUE_LEN` | 51 | not sent: the label of 19 bytes and the responder's identity key, hashed into the handshake by both sides |
| `HANDSHAKE_MSG1_LEN` | 48 | fixed: an ephemeral key and a tag |
| `HANDSHAKE_MSG2_LEN` | 48 | fixed: an ephemeral key and a tag |
| `HANDSHAKE_MSG3_LEN` | 235 | fixed: the initiator's transport key and its contact card, each encrypted, each with a tag |
| `FRAME_LENGTH_PREFIX_LEN` | 2 | big-endian |
| `FRAME_PADDING_BLOCK_LEN` | 1024 | plaintext is a multiple of this; provisional value, see ADR 0003 |
| `MIN_FRAME_CIPHERTEXT_LEN` | 1040 | one block plus tag |
| `MAX_FRAME_CIPHERTEXT_LEN` | 64528 | 63 blocks plus tag |
| `MAX_FRAME_PLAINTEXT_LEN` | 64512 | |
| `MESSAGE_HEADER_LEN` | 4 | type and body length |
| `MAX_MESSAGE_BODY_LEN` | 64508 | |

The handshake has no variable-size message, no length field and no
preamble (ADR 0002). A peer that has not completed the handshake can make
Monolith read at most 48 bytes (as initiator) or 48 + 235 bytes (as
responder), and the second of those only after a first message that
verified. Each message is read into an array of exactly its size.

The frame ceiling follows from the session cipher: a Noise message is at most
65535 bytes. 63 padding blocks plus the tag is the largest multiple that fits.

State-dependent ceiling: in `AuthenticatedUnknown` the only legal messages
(ContactRequest, ContactAccept, Close) fit in one padding block, so in that
state the frame length must be exactly 1040. Before that state no frame is
accepted at all. A peer gets the full frame size only after the session is
confirmed as a contact session.

## 2. Field sizes (protocol)

| Constant | Value | Initially proposed | Reason for the difference |
| --- | --- | --- | --- |
| `MAX_CHAT_TEXT_LEN` | 16384 | 32768 | Bounds the UI event queue at about 4 MiB instead of 8 MiB. 16 KiB is several pages of text. |
| `MAX_INTRODUCTION_TEXT_LEN` | 512 | 1024 | Text from an unknown peer; keeps a contact request inside one padding block. |
| `MAX_DISPLAY_NAME_LEN` | 128 bytes | 128 | Additionally at most 64 scalar values. |
| `MAX_PROFILE_TEXT_LEN` | 1024 | 2048 | No use for more in v1. |
| `MAX_FILENAME_LEN` | 255 | 255 | |
| `MAX_ACTIVE_ENDPOINTS` | 1 | - | Endpoints in one contact card. The format allows a set; version 1 allows one member. |
| `MAX_CONTACT_CARD_LEN` | 187 | 8192 | Fixed layout: 171 bytes with one endpoint, or 187 with an invitation capability. |
| endpoint update | 171 | 4096 | An endpoint update is a contact card without a capability. |
| `MAX_FILE_CHUNK_LEN` | 64490 | 65536 | A 64 KiB chunk cannot fit in a frame that is itself capped at 64 KiB. This is the largest chunk that fills a maximum frame. |
| handshake message | 48 / 48 / 235 | 8192 | Fixed sizes. |
| `MAX_CONTACT_CARD_TEXT_LEN` | 512 bytes | - | Input bound for the textual card parser; the longest valid card is 310 characters, and whitespace may be mixed in. |
| `MAX_FILE_SIZE` | 4 GiB | - | Largest size an offer may declare on the wire, and the ceiling of the transfer store format. The field is 64 bits wide. Since Phase 8 the size Monolith sends or accepts is `MAX_TRANSFER_SIZE` (section 13). |
| `MAX_TRANSFER_SIZE` | 256 MiB | - | Local limit of a file sent or accepted, since Phase 8. It replaces `DEFAULT_MAX_FILE_SIZE` (1 GiB), which no longer exists. An offer above it is answered with FileReject. |

Identifiers: message and transfer identifiers are 16 random bytes, invitation
capabilities are 16 random bytes, ping nonces are 8 bytes.

## 3. Session lifetime (protocol)

| Constant | Value |
| --- | --- |
| `MAX_SESSION_LIFETIME` | 24 hours |
| `MAX_SESSION_LIFETIME_WITH_TRANSFER` | 48 hours |
| `MAX_FRAMES_PER_DIRECTION` | 2^32 |
| `MAX_BYTES_PER_DIRECTION` | 2^40 |
| `SESSION_CLOSE_GRACE` | 120 seconds |

A session that reaches a limit is closed and replaced by a new handshake.
There is no in-band rekey; see `docs/CRYPTOGRAPHY.md` section 7. The nonce of
the session cipher is a 64-bit counter, so the frame limit leaves a factor of
2^32 before nonce exhaustion. At the maximum frame size the frame limit
alone would allow more than 2^40 bytes, so the byte limit is the one that
binds for bulk transfer: 1 TiB per direction per session.

How the limits are applied, by the session object itself
(`PROTOCOL.md` section 7.1):

- The earliest limit that is reached ends the session: age, frames in one
  direction, or ciphertext bytes in one direction.
- Frames and bytes are counted per direction. The bytes are the ciphertext
  of each frame, tag included, without the 2-byte length prefix. Plaintext
  is always less than that, so the byte limit bounds it as well.
- Sending: a message is refused when, after it, no room would be left for
  a Close. One frame and 1040 bytes are kept back for the Close, which is
  the only thing that can still be sent at the limit. At the age limit
  nothing but Close is sent.
- Receiving: a frame beyond the frame or byte limit is a protocol
  violation. So is a frame that arrives once `SESSION_CLOSE_GRACE` has
  passed after the age limit. The stream is closed and nothing is sent.
- The age is measured from the moment the handshake completed, on each
  side's own clock. The two clocks start up to one handshake apart, which
  `HANDSHAKE_TIMEOUT` bounds at 30 seconds, and a Close that is sent at the
  limit needs time to be written and to arrive. The grace covers both. It
  is the reason why the orderly end of a long session is not counted as a
  violation by the side whose clock is ahead.
- The age is measured with a monotonic clock. On Linux that clock does not
  advance while the machine is suspended, so the limits are in running
  time. A Tor stream rarely outlives a suspend.
- The limits are constants. No configuration changes them, in either
  direction: both ends of a session have to agree on them, and they are
  not negotiated. A local policy that wants shorter sessions closes them
  earlier. Tests of the session layer use lower values through a
  constructor that exists only in test builds.

A session that reaches 24 hours while a file transfer is active stays open
until the transfer ends, starts no new transfer, and is closed at 48 hours
whatever happens. A transfer that would begin after 24 hours does not
extend the session. A 4 GiB file needs under 50 KiB/s to finish in a day.

Phase 8 does not use the longer lifetime. A session ends at
`MAX_SESSION_LIFETIME` whatever its transfers do, and they go on over the
next session (`adr/0013-file-transfer.md`). `MAX_SESSION_LIFETIME_WITH_TRANSFER`
stays a constant of the protocol.

## 4. Timeouts (local)

| Constant | Value | Purpose |
| --- | --- | --- |
| `CONNECT_TIMEOUT` | 120 s | Outbound stream through Tor. Onion connections need a descriptor fetch, an introduction and a rendezvous. |
| `HANDSHAKE_TIMEOUT` | 30 s | From stream open to an authenticated state. Covers three messages over an established circuit. This is the slowloris bound for unauthenticated streams: a handshake message that arrives later is refused, and a peer that sends a valid first message and then nothing holds its pending handshake for this long. |
| `UNKNOWN_SESSION_TIMEOUT` | 20 s | Longest time in `AuthenticatedUnknown`, measured from authentication. Then the session is closed with Close. |
| `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | 10 s | A peer that is not a contact for the session must send its first message within this time, measured from authentication. Then the session is closed with Close. |
| `DUPLICATE_PROBE_TIMEOUT` | 10 s | Liveness probe of an older session during duplicate resolution. Counts only while the session is read (section 6). |
| `PING_INTERVAL_MIN` / `MAX` | 90 s / 150 s | Each interval is drawn uniformly from this range. |
| `PONG_TIMEOUT` | 60 s | A Ping not answered for this long ends the session. Counts only while the session is read (section 6). |
| `IDLE_TIMEOUT` | 240 s | No complete frame received, measured from the last complete frame or, before the first, from authentication. Bytes of a frame that is not complete do not count. The session ends without a Close. |
| `MESSAGE_ACK_TIMEOUT` | 60 s | A sender waits this long for the MessageAck of the message in flight. Then delivery is uncertain: the session is closed and the message is sent again on a later one (`adr/0007-delivery-semantics.md`). |
| `SESSION_LINGER` | 120 s | A confirmed session on which, for this long, no ChatMessage was sent or taken in and no MessageAck delivered a message, with nothing queued or in flight, is closed by the local side with Close (`adr/0009-session-manager.md`). A message held already and an acknowledgement of nothing do not count. |
| `FRAME_READ_TIMEOUT` | 60 s | From the first byte of a frame's length prefix to its last byte, an absolute deadline: later bytes of the same frame do not move it. The session ends without a Close. |
| `FRAME_WRITE_TIMEOUT` | 60 s | One frame write. |
| `FILE_OFFER_TIMEOUT` | 24 hours | An incoming offer the user has not decided expires after this, across sessions and restarts (it was 10 minutes while an offer lived and died with its session). The timeouts of transfers are in section 13. |
| `CONTROL_COMMAND_TIMEOUT` | 30 s | One Tor control command. |
| `CONTROL_CONNECT_TIMEOUT` | 10 s | Opening a control connection and authenticating on it. |
| `SOCKS_NEGOTIATION_TIMEOUT` | 10 s | Reaching the SOCKS endpoint and its greeting and authentication replies. The CONNECT reply, which waits for the Onion Service, has `CONNECT_TIMEOUT`. |
| `SHUTDOWN_TIMEOUT` | 5 s | `DEL_ONION` when a published service is closed; closing the control connection removes the service anyway. |
| `ACCEPT_BACKOFF` | 250 ms | Pause of an accept loop after it closed a stream for which no handshake slot was free. |

The timeouts are separate on purpose. A Tor that is still bootstrapping is
not an error of any of them: the status query reports it, and the
reconnect schedule of section 7 decides when to try again.

After the handshake, `AuthenticatedSession::deadline` gives the earliest
of the deadlines that apply to a session: the frame that has begun, the
idle limit, the age limit of section 3, while the session is in
`AuthenticatedUnknown` the two unknown-session limits, and a Close the
session logic decided after the first message of a peer that is not a
contact, which is due at once. `link::Link` waits for that moment
alongside the stream and a withdrawal, so each deadline fires while bytes
are awaited and does not depend on a frame completing; the Close after a
first message it writes before it returns the message. The frame and idle
deadlines end the session without a Close; the others end it with one, as
the local side ends a session on purpose. Whatever ends a link, it gives
back its slot of `MAX_UNKNOWN_SESSIONS` as soon as the Close, if any, is
written, within `FRAME_WRITE_TIMEOUT`.

## 5. Concurrency budgets (local)

| Constant | Value | Notes |
| --- | --- | --- |
| `MAX_INBOUND_HANDSHAKES` | 16 | Streams accepted from the Onion Service that are not yet authenticated. |
| `MAX_UNKNOWN_SESSIONS` | 4 | Authenticated peers with no contact record, or declined or blocked, and contacts with a stale card or a pending key for the session. |
| `MAX_CONTACT_SESSIONS` | 256 | Identities held as accepted or requested, inbound and outbound, confirmed or not. |
| `MAX_CONCURRENT_DIALS` | 4 | Outbound connection attempts. |
| `MAX_CONTACTS` | 1000 | |
| `MAX_BLOCKED_IDENTITIES` | 10000 | 32 bytes each. |
| `MAX_DECLINED_IDENTITIES` | 1024 | Oldest dropped when full. |
| `MAX_PENDING_CONTACT_REQUESTS` | 32 | One per identity. |
| `MAX_PENDING_REQUESTS_PER_INVITATION` | 8 | Per capability, or for requests without one. |
| `MAX_ACTIVE_INVITATIONS` | 16 | Size of the active set (PROTOCOL.md 12.3). Kept in Phase 4, see below. |
| `MAX_UI_EVENTS` | 256 | Coalesced, see section 8. |
| `MAX_PENDING_FILE_OFFERS_PER_CONTACT` | 4 | Incoming offers waiting for the user. |
| `MAX_PENDING_FILE_OFFERS` | 16 | The same, per local identity since Phase 8. |
| `MAX_ACTIVE_TRANSFERS_PER_CONTACT` | 2 | Incoming transfers accepted and not complete, per contact. |
| `MAX_ACTIVE_TRANSFERS` | 4 | The same, per local identity since Phase 8. |
| `MAX_ENDED_TRANSFER_IDS` | 64 | Per session; lets messages that crossed an abort be discarded. |
| `FILE_FREE_SPACE_MARGIN` | 64 MiB | Free space required beyond the declared size. |
| `MAX_QUEUED_MESSAGES_PER_CONTACT` | 256 | Outbound, not yet acknowledged, per contact record. |
| `MAX_QUEUED_MESSAGES_PER_IDENTITY` | 1024 | Outbound, not yet acknowledged, all contacts of one local identity. |
| `MAX_QUEUED_MESSAGE_BYTES` | 8 MiB | Text queued, all identities together: half of the message store, so that a full queue leaves room to take messages in. |
| `MESSAGE_DEDUP_WINDOW` | 1024 | Identifiers of received messages the user deleted, kept per conversation so that a late retry is not taken in again. Messages that are held are found by their own identifiers. |
| `MAX_CONVERSATIONS_PER_IDENTITY` | 2048 | Conversations in the message store, those of deleted or blocked contacts included. |
| `MAX_CONVERSATION_STORE_LEN` | 4 MiB | Bytes the messages of one conversation take in the message store; one contact cannot fill it. |
| `MAX_STORED_MESSAGES` | 262144 | Messages in the message store, all conversations together. |

Tor applies one limit of its own in front of these: `MaxStreams=8` with
`MaxStreamsCloseCircuit` on every `ADD_ONION` (`TOR_CONTROL_SURFACE.md`). It
counts streams on one rendezvous circuit, not streams to the service as a
whole, and a circuit that goes over it is closed. It is defense in depth
only; the budgets above decide what Monolith accepts. The value is
provisional; its review with the real connection model and measurements
(`DESIGN_QUESTIONS.md` T3-6, C1) is still open after Phase 4, which did not
change it.

The handshake budget and the budget for strangers are two budgets. The
first counts streams that have not authenticated, whose peer is not known
yet; the second counts authenticated peers that are not contacts. When the
handshake budget is full, the new stream is closed at accept and the accept
loop pauses for `ACCEPT_BACKOFF`; no held stream is closed for it. A flood
can therefore keep the 16 slots full, each stream for at most
`HANDSHAKE_TIMEOUT`, and a connection gets through only when it finds a
free slot. Closing the oldest unauthenticated stream for a new one, so that
a flood churns the slots instead of pinning them, is not implemented yet
(section 12).
Inbound floods do not affect outbound dials, which have their own budget, so
two contacts can still reach each other as long as one of the two Onion
Services is reachable.

When the budget for strangers is full and another one authenticates, the
oldest that has not yet sent its message is closed with a Close, and the
newcomer takes its slot (`crate::strangers` in the core, since Phase 4). A
stranger that has sent its message is not evicted: its message and the
eviction race on one state, and whichever comes first decides, so an
evicted stranger's message is never delivered. If every held slot belongs
to a stranger that has spoken, the newcomer is closed without anything
sent. Evicted links that have not ended yet are bounded by the same
number, so at most twice the budget exist at any moment; when that bound
is reached the newcomer is closed as well. A contact is never in this
budget and never evicted for a stranger. The deadlines of section 4 bound
how long a slot is held: `UNKNOWN_FIRST_MESSAGE_TIMEOUT` for a stranger
that sends nothing, and for one that does, until its first message and
the Close after it, written within `FRAME_WRITE_TIMEOUT`.

When the contact session budget is full, no further contact session is
accepted or dialed until one ends: an inbound contact is closed after its
admission, and a dial ends before message 3 is written. With more than 256
contacts online at the same time, some stay unreachable. The value is a
starting point to be measured, not a claim that 256 is enough for 1000
contacts.

When the pending-request queue is full, new requests are dropped and the user
is told once that the queue is full. Existing entries are not evicted; an
attacker must not be able to push a legitimate request out. At most 8
pending requests may share one invitation, so a leaked or published
invitation cannot fill the queue. A capability is reusable (PROTOCOL.md
12.2), so a card in a public directory can bring in requests faster than
the user handles them; from the ninth pending one on, its requests are
dropped like any other, and their senders retry on their next session.

The credentials of a contact are bounded by their definition
(PROTOCOL.md 11.4): one active card, at most one authorized and one
pending successor, one retired key, two cards of at most 187 bytes each
beyond the active one. A peer cannot grow them: a newer announcement
replaces the authorized successor and a newer pending card replaces the
pending one. A session of a retired key is withdrawn and frees its slot.
Each link carries one withdrawal handle, a flag and a wakeup, freed with
the link.

The active set of invitation capabilities is bounded by
`MAX_ACTIVE_INVITATIONS`. Each request that carries a capability is
compared with every member, so the bound is also the work per request:
16 comparisons of 16 bytes. The set itself takes 256 bytes plus the local
labels. When it is full, a new capability can be created only after one
is revoked; nothing is revoked automatically. 16 is the value of Phase 0,
kept in Phase 4 (`DESIGN_QUESTIONS.md` P4-12): it bounds the work per
request and the size of the vault, and the contact store gave no reason
to change it. It has not been checked against how many cards users keep
in circulation.

### 5.1 Several local identities

Every value above is written for one local identity in one process. With
several (`ARCHITECTURE.md` section 1.1), a per-identity value must not be
multiplied by the number of identities without bound. Budgets fall into
three levels:

    process-wide     a ceiling for the whole process
      per identity   what one identity's peers can use
        per contact  what one peer of one identity can use

For example: inbound sockets of the process above the handshake slots of
one identity, above the requests of one peer.

- Per identity: what a peer can observe, so that load on one identity is
  not visible through another (`THREAT_MODEL.md` adversary S). That covers
  the inbound handshake slots, the budget for strangers, the pending
  request queue and its quota per capability, the active set of
  capabilities, the contact sessions, and the inbound rates of section 6.
- Process-wide: ceilings on the totals, such as open streams, memory,
  dials and transfers, sized so that they bind only when the process as a
  whole is under pressure. When one binds, its effect is shared by every
  identity, and that link is a residual risk.
- Per contact: the limits that are per contact today stay per pair of
  local and remote identity.

The values of Phase 4 (`DESIGN_QUESTIONS.md` P4-7), provisional like the
others:

| Constant | Value | Level |
| --- | --- | --- |
| `MAX_LOCAL_IDENTITIES` | 8 | process |
| `MAX_INBOUND_HANDSHAKES` | 16 | identity |
| `MAX_PROCESS_INBOUND_HANDSHAKES` | 32 | process |
| `MAX_UNKNOWN_SESSIONS` | 4 | identity |
| `MAX_CONTACT_SESSIONS` | 256 | identity |
| `MAX_PROCESS_CONTACT_SESSIONS` | 512 | process |
| `MAX_CONCURRENT_DIALS` | 4 | process |
| `MAX_PENDING_CONTACT_REQUESTS`, `MAX_PENDING_REQUESTS_PER_INVITATION` | 32, 8 | identity |
| `MAX_ACTIVE_INVITATIONS` | 16 | identity |
| `MAX_CONTACTS`, `MAX_BLOCKED_IDENTITIES`, `MAX_DECLINED_IDENTITIES` | as above | identity |

An inbound handshake takes a slot of its identity and one of the process,
and so does a contact session; `Budgets` holds the process ceilings and
each `LocalIdentity` its own. The budget for strangers, the request queue
and the active set exist per identity only. A process ceiling binds only
when several identities are busy at once; when it does, the refusal is
shared by all of them, which is the residual of `THREAT_MODEL.md`
adversary S.

What section 11 becomes with 8 identities: contact sessions are bounded by
the process ceiling, 512 sessions or about 64 MiB of buffers; handshakes
by 32; strangers by 4 per identity with as many evicted ones still
draining, 64 links in all; pending requests by 256 in all; each identity
has its control connection and listener. The contact lists are per
identity, so 8 identities may hold 8000 contacts, and the duplicate
windows of section 11, which the session manager will keep per contact,
could then reach about 125 MiB. A process-wide ceiling on contacts, or on
the windows, is not set yet; it comes with the session manager
(`DESIGN_QUESTIONS.md` section 11.3).

## 6. Rates (local)

Token buckets. "Burst" is the capacity, "per minute" the refill.

| Constant | Burst | Per minute | Scope | Enforced |
| --- | --- | --- | --- | --- |
| `INBOUND_CONNECTION_RATE` | 32 | 120 | All inbound streams. | no, see below |
| `UNKNOWN_SESSION_RATE` | 8 | 12 | All unknown identities together. | no, see below |
| `CHAT_MESSAGE_RATE` | 60 | 300 | Per contact. | since Phase 7 |
| `CONTROL_MESSAGE_RATE` | 120 | 600 | Per contact. Ping, Pong, MessageAck, EndpointUpdate, file control messages, and ContactRequest or ContactAccept received after confirmation. | since Phase 7 |
| `FILE_OFFER_RATE` | 4 | 6 | Per contact. | since Phase 7 |
| `PROFILE_UPDATE_RATE` | 8 | 8 | Per contact. Not below the session rate, because a profile is sent once per session. | since Phase 7 |
| `CONTACT_SESSION_RATE` | 6 | 6 | New authenticated sessions per identity. | since Phase 7, inbound sessions |

FileChunk is not rate limited by count. It is limited by the declared size
of an accepted transfer and by backpressure: Monolith stops reading from a
session while the disk writer or the UI queue is behind. Since Phase 8
FileOffer counts against `FILE_OFFER_RATE` and the other file messages
against `CONTROL_MESSAGE_RATE`; the bounds on chunks are in section 13.

How the rates are enforced since Phase 7 (`crate::rates` in the core,
`DESIGN_QUESTIONS.md` P7-8):

- A bucket starts full, gives its burst at once and then one token every
  60 s divided by its rate per minute. It is kept as one instant, the
  moment it would be full again, so it needs no timer; a bucket that is
  full holds nothing and is dropped.
- The four message rates are per contact record, across all its
  sessions, so a contact that reconnects does not start over, and count
  what arrives on a confirmed session. `CONTACT_SESSION_RATE` is per
  remote identity and counts inbound sessions whose standing is that of a
  contact record; a session this side dialed is not counted. Every bucket
  belongs to one local identity, and none is keyed by anything but what a
  session authenticated: under Tor there is no address to key by, and
  none is used.
- The message rates are enforced by waiting, not by disconnecting: when a
  bucket is empty, the session waits for its token before it handles the
  message and reads nothing meanwhile. A conforming sender therefore does
  not need to know the rates, and a sender that resends a full queue
  after a reconnect is slowed down, not cut off. A session that is closed
  while it waits ends at once, and the message is not handled; its
  sender keeps it. A session opened above `CONTACT_SESSION_RATE` is closed
  with Close once it is admitted, before its first message.
- What one contact sends or opens never slows another: no bucket is
  shared between contacts.
- The buckets live in memory. A restart starts every one full, which
  gives a contact one burst more; a peer cannot make Monolith restart.
- Memory: one instant for each bucket that is not full. When those of
  `MAX_CONTACTS` contacts are held, the full ones are dropped before
  another is added; a record that ends drops its message buckets. The
  bucket of the sessions a contact opens is keyed by its identity key and
  stays, so that deleting and adding a contact again does not refill it.

The rates of streams and of strangers, `INBOUND_CONNECTION_RATE` and
`UNKNOWN_SESSION_RATE`, are not enforced. Before authentication nothing
tells one peer from another, and after it a fresh identity key costs a
stranger nothing, so each could only be one bucket for everyone, which a
single peer that keeps it empty would close to every other: for streams
that is every contact that dials in as well. What they would bound is
bounded already: streams by the handshake budget of section 5 and
`HANDSHAKE_TIMEOUT`, at the cost of section 11.1; strangers by their
budget, the eviction of the oldest silent one and the quotas of the
request queue, which keep requests with a capability apart from those
without (one bucket for all strangers would let requests without a
capability shut out every invitation).

While Monolith is not reading from a session it cannot answer that peer's
Pings and does not see its Pongs. Its own Pong timer is suspended for that
time. The peer's is not, so a session whose reader stays paused for longer
than `PONG_TIMEOUT` is ended by the other side. That is intended: a
receiver that cannot keep up for a minute gives up the session and does not
buffer.

## 7. Reconnect schedule (local)

| Constant | Value |
| --- | --- |
| `RECONNECT_DELAY_INITIAL` | 10 s |
| `RECONNECT_BACKOFF_FACTOR` | 2 |
| `RECONNECT_DELAY_MAX` | 30 min |
| `RECONNECT_JITTER_PERCENT` | 50 |
| `RECONNECT_RESET_AFTER` | 60 s |
| `STARTUP_DIAL_SPREAD` | 60 s |

Delay sequence before jitter: 10 s, 20 s, 40 s, ... capped at 30 min. Each
delay is multiplied by a uniform random factor in [0.5, 1.5], in steps of
0.01, each as likely as the others (since Phase 7, P7-12). The delay is
reset after a session has stayed authenticated for 60 s.

Since Phase 4 the publication supervisor uses the same schedule to publish
the service of an identity again after Tor lost it: the first attempt
follows the loss after the first delay, each failure doubles it, and the
delay is reset once a publication stayed up for `RECONNECT_RESET_AFTER`.
There is no attempt without a delay.

Since Phase 5 the same schedule paces the dialing of contacts, and only of
a contact that has queued messages and no session
(`adr/0009-session-manager.md`): the first attempt follows the command
that queued the first message, the next ones the delays above after a
failure or after a session that ended with messages left, and the delay
starts over once a session of the contact stayed up for
`RECONNECT_RESET_AFTER`. At startup the first attempts are spread at random
over `STARTUP_DIAL_SPREAD`. A contact with nothing queued is not dialed.

Since Phase 8 the same schedule also runs while a transfer needs a
session, and a dial made for transfers alone resets the delays only if
the session brought progress (section 13.4).

With 1000 offline contacts at the cap, a dial comes due about every 1.8 s
on average. Only 4 run at once. A dial that is due waits for a free slot in
order of due time, so when dials are slow (the timeout is 120 s) the retry
interval stretches; the number of concurrent attempts does not grow.
Because both sides dial, a contact that comes online reaches us promptly
even when our own retry timer is long.

## 8. UI event queue

The core sends events to a front end through one bounded queue of
`MAX_UI_EVENTS` entries. Since Phase 5 the runtime keeps this as a bounded
broadcast: events name what changed (a message by its conversation and
place, never its text), and one that reports stored state is raised once
the change is durable (ADR 0007); a front end that falls more than
`MAX_UI_EVENTS` behind is told how many it missed and reads the state
again with the queries. Nothing waits for a slow front
end, and nothing is lost by it: the state is in the message store. What
follows was the design before Phase 5 and still describes the events of
the contact store. Events that describe state (peer online, request
count, transfer progress) are coalesced: a newer event of the same kind for
the same subject replaces the queued one. Events that carry content (a chat
message) are not coalesced; when the queue is full the core stops reading
from the sessions that would produce more. A slow front end slows peers
down. It does not grow memory.

Notifications are counted, not listed: "3 new contact requests", never one
window or one desktop notification per request.

## 9. Local Tor replies and storage headers

| Constant | Value |
| --- | --- |
| `MAX_CONTROL_LINE_LEN` | 1024 |
| `MAX_CONTROL_REPLY_LINES` | 16 |
| `MAX_CONTROL_REPLY_LEN` | 4096 |
| `MAX_SOCKS_REPLY_LEN` | 262 |
| `ONION_MAX_STREAMS` | 8, per rendezvous circuit (section 5), provisional |
| `MAX_VAULT_FILE_LEN` | 16 MiB |
| `MAX_MESSAGE_STORE_FILE_LEN` | 16 MiB |
| `MESSAGE_STORE_PADDING_BLOCK_LEN` | 64 KiB |
| `MIN_KDF_MEMORY_KIB` / `DEFAULT` / `MAX` | 64 MiB / 256 MiB / 1 GiB |
| `MIN_KDF_ITERATIONS` / `DEFAULT` / `MAX` | 3 / 3 / 16 |
| `DEFAULT_KDF_PARALLELISM` / `MAX` | 4 / 8 |
| `VAULT_PADDING_BLOCK_LEN` | 64 KiB |
| `MAX_PASSPHRASE_LEN` | 1024 |

The KDF defaults are provisional until they have been benchmarked on the
target platforms (`STORAGE.md` section 3.3). The accepted range is not.

Tor is trusted more than a peer, but its replies are parsed with bounds all
the same. The vault header is bounded because whoever can write to the disk
could otherwise set a KDF memory cost that exhausts RAM at unlock.

## 10. Rendering limits

Independent of the protocol limits. They apply to what a front end draws.

| Constant | Value |
| --- | --- |
| `MAX_RENDERED_GRAPHEME_SCALARS` | 16 |
| `MAX_RENDERED_UNBROKEN_RUN` | 64 |
| `MAX_RENDERED_MESSAGE_LINES` | 200 |

A grapheme cluster with more scalar values than the limit is drawn as a
placeholder. A run without a break opportunity is broken by the renderer.
A message with more lines than the limit is collapsed behind an explicit
"show more" action.

### 10.1 The desktop client

Since Phase 6 (`DESKTOP.md`). Local limits of the client; none changes what
the runtime accepts.

| Limit | Value | Where |
| --- | --- | --- |
| Commands and updates waiting between the window and the bridge | 256 each way | `bridge.rs`; a command that does not fit waits in the window's queue, an update in the bridge |
| Jobs waiting for the disk thread | 16 | `files.rs`; one more fails at once |
| A card file read | 64 KiB, a regular file | `files.rs` |
| The settings file read | 64 KiB; a value 4096 bytes, one line | `config.rs` |
| A fallback font | 32 MiB | `system_fonts.rs` |
| Messages read at a time | 100; at most 2000 in a window | `monolith-ui-model` (`PAGE`, `MAX_WINDOW`) |
| A preview in the list | 120 characters laid out | `sidebar.rs` |
| A run without a break opportunity | broken where it meets the width of its bubble or box | `bubble.rs`, the text layout of egui; nothing is inserted into the text, so what is copied is what was sent (P7-14) |
| A local name typed | 64 characters and 128 bytes, those of `display_name` | `dialogs.rs`, `present.rs` (P7-14) |
| Notices shown at once | 3, each 5 seconds unless it is an error | `toast.rs` |
| A QR code | a card of at most 1200 bytes | `qr.rs`; a longer one is shown as text only |

## 11. Worst-case budget

Memory that a peer can influence, with every budget full:

| Item | Bound |
| --- | --- |
| Contact sessions: 256 x (read buffer + write buffer, one frame each) | about 32 MiB |
| Pending handshakes: 16 x about 3 KiB (section 11.1) | about 48 KiB |
| Unconfirmed sessions: one frame of one block each, plus about 2 KiB of session state | under 1 MiB |
| Message store: the state in memory, and at a write a copy of it, its encoding and the sealed file (measured, P7-20) | 16 to 46 MiB between writes, more for many short messages; 64 to 100 MiB at a write |
| UI event queue: 256 events, none of them with message text | under 0.1 MiB |
| Active transfers: 4 x (one chunk + hash state) | about 0.3 MiB |
| Pending requests, file offers | under 0.1 MiB |
| Contacts with profiles, block and declined lists | about 2 MiB |
| Total | about 136 MiB |

The total does not include what an ephemeral installation stages in memory,
up to `MAX_EPHEMERAL_STAGED_BYTES`, 256 MiB (section 13.1).

The message store holds what the user queued and what contacts sent, up to
its bound (`MAX_MESSAGE_STORE_FILE_LEN`); a contact fills at most its
conversation (`MAX_CONVERSATION_STORE_LEN`). Full-size session buffers are
allocated when a session is confirmed as a contact session and freed when
it closes, so the typical footprint is far lower.

The identifiers that drop duplicates are those of the messages the store
holds and, per conversation, at most `MESSAGE_DEDUP_WINDOW` of received
messages the user deleted; they outlive sessions and restarts.

Not peer-influenced: the Argon2id working memory at vault unlock (256 MiB by
default, once, then released) and whatever the GUI toolkit uses.

Other resources:

- File descriptors: at most 16 + 4 + 256 session sockets, 4 dials, 1 control
  connection, 1 listener, 4 transfer files, a few storage files. Under 300,
  below the usual soft limit of 1024.
- Tasks: one per session and per dial, one writer per active transfer, and a
  fixed set of supervisors. Under 300. No task is spawned per message or
  per frame, and none is detached from its supervisor.
- Disk: an incoming transfer is accepted only if free space is at least the
  declared size plus `FILE_FREE_SPACE_MARGIN`. At most 4 transfers of at most
  `MAX_TRANSFER_SIZE` each are in progress, and the staged bytes are bound
  as section 13.2 says. The vault is capped at
  16 MiB, and so is the message store (`MAX_MESSAGE_STORE_FILE_LEN`);
  each is rewritten whole, under another name, so a write needs as much
  again while it runs.
- CPU: an inbound handshake costs three X25519 operations, two fixed-base
  multiplications and the validation of one contact card, about 0.3 ms
  (section 11.1). Streams are not rate limited (section 6); the
  handshake budget bounds how many are under way, and at 120 a minute a
  stream would cost a fraction of a percent of one core.
- A failed dial under the SOCKS proof (`SocksProof::ControlListeners`)
  ends the proof, so the next dial makes it again: one control connection
  to Tor, with SAFECOOKIE authentication and one `GETINFO`. Measured on
  loopback against the fake control port of the tests, release build, the
  proof adds about 1.8 ms to a failed dial (2.8 ms against 1.0 ms without
  it); a dial that fails over Tor takes seconds. Dials are paced by the
  delays of section 7 and `MAX_CONCURRENT_DIALS`, so a proof is made at
  most as often as a dial fails (P7-21).

### 11.1 Cost of a handshake

Measured with a throwaway program that is not in the repository, release
build, one pinned core, with the key types and the resolver of
`monolith-session` and keys from the operating system. Times are medians
of 2000 handshakes; memory comes from a counting global allocator. First
measured before the review fixes of Phase 2, and again after them on
another development machine (Intel Core i5-11400), where the memory
figures were the same and the times a little lower; the table gives the
second measurement. The numbers say what the order of magnitude is. They
are not a benchmark, and other hardware will differ.

What a responder does for one inbound stream, by how far the peer gets:

| The peer sends | Asymmetric operations | Time | Reply |
| --- | --- | --- | --- |
| nothing, or fewer than 48 bytes | one fixed-base multiplication, when the handshake object is created | 0.02 ms | none; closed at `HANDSHAKE_TIMEOUT` |
| 48 bytes whose key is not valid | the same | 0.02 ms | none |
| 48 bytes that fail authentication | plus one X25519 | 0.06 ms | none |
| a first message that verifies, which needs the contact card | plus one key generation and one more X25519 | 0.12 ms | 48 bytes |
| then a valid third message | plus one X25519, one Ed25519 verification and the subgroup checks of two keys | 0.16 ms more | none; the session exists |

An initiator spends about 0.07 ms on the first message and 0.09 ms between
the second and the third. A complete handshake costs both sides together
about 0.45 ms. Sealing and opening one frame of one block takes about
0.004 ms.

Memory and bytes:

| Item | Value |
| --- | --- |
| Bytes on the wire for a handshake | 48 + 48 + 235 = 331 |
| State of one pending inbound handshake | about 2.7 KiB: an object of 2072 bytes and 665 bytes on the heap |
| The same after a valid first message | unchanged |
| Peak heap while the third message is processed | 275 bytes more |
| Allocations for a whole inbound handshake | 15, the creation of the session included, none sized by anything the peer sent |
| State of one session, without frame buffers | about 2 KiB: an object of 1888 bytes and 66 bytes on the heap |

Consequences:

- A caller without the contact card can cost a responder one X25519
  operation per stream, 0.06 ms, and gets no protocol response. At 120
  streams a minute that would be under 10 ms of processor time a minute;
  no inbound rate is enforced (section 6), so the bound is how fast Tor
  brings streams, 16 at a time at most.
- A caller with the contact card can make the responder do the full
  handshake, about 0.3 ms, and can hold a pending handshake of 2.7 KiB for
  30 seconds. `MAX_INBOUND_HANDSHAKES` bounds the second at 16 x 2.7 KiB.
- The invitation capability is looked at only inside a ContactRequest,
  after a complete handshake. A holder of a card who tries capabilities
  pays a full handshake and one request per attempt, and is bounded by
  the budget for unknown sessions; a capability has 128 bits and is not
  found by trying. Every attempt is answered with the same Close.
- The responder does no signature and no work proportional to anything in
  a message.

### 11.2 Cost of the Tor adapter

Measured on the development machine of section 11.1 with a throwaway
program that is not in the repository: release build, the SOCKS and
control servers in a separate process so that only Monolith's own
allocations are counted, after one warm-up of the runtime. The machine was
busy with other work at the time, so the times give the order of
magnitude only.

| What | Allocations | Peak heap | Time |
| --- | --- | --- | --- |
| Parsing and interpreting a `PROTOCOLINFO` reply (133 bytes) | 13 | 2.3 KiB | 1.4 us |
| Parsing and interpreting an `ADD_ONION` reply (196 bytes) | 10 | 2.4 KiB | 37 us, most of it the key checks of the ServiceID |
| Decoding a SOCKS CONNECT reply | 0 | 0 | 15 ns |
| A SOCKS negotiation to an onion service | 10 | 256 bytes | |
| A status query: control connection, SAFECOOKIE, two GETINFO, SOCKS greeting | 40 | 2.7 KiB | |
| A publication | 41 | 2.8 KiB; 1.2 KiB held while the service is published | |
| `close` with `DEL_ONION` | 5 | 1.4 KiB | |
| An accepted inbound socket before its handshake | 2 | registration only; the stream value is 32 bytes | |

What a peer can make Monolith hold, by the bounds rather than by the
measurements:

- A control reply is at most `MAX_CONTROL_REPLY_LEN`, 4096 bytes, in
  lines of at most `MAX_CONTROL_LINE_LEN`; the parser's line buffer is
  allocated once per connection at 1028 bytes. A SOCKS reply is read into
  a fixed 262-byte array on the stack. Only Tor, not a peer, sends either.
- Inbound streams that have not authenticated: at most
  `MAX_INBOUND_HANDSHAKES`, 16, each with its socket, its task and a
  pending handshake of about 2.7 KiB (section 11.1). A stream beyond that
  is closed at once, and the accept loop pauses for `ACCEPT_BACKOFF`.
- Outbound SOCKS negotiations: at most `MAX_CONCURRENT_DIALS`, 4.
- Control connections: one per published service, held while it is
  published, and one per status query while it runs. They are made by the
  local side, never by a peer.
- An authenticated session's link reads into one 4096-byte buffer and reads
  again only when the session has taken it.

## 12. What the limits do not prevent

An attacker who knows the Onion Service address can keep the handshake
budget full. Legitimate inbound connections then get through only when
they find a free slot, until the flood stops; dials of this side are not
affected (section 5). Tor's proof-of-work defense, where it is active, and the cap on
streams per circuit act before Monolith sees a stream; see
`docs/TOR_INTEGRATION.md`. Monolith does not claim to defeat targeted denial
of service against an Onion Service.

Since Phase 3 the network layer of the core enforces the budgets for
inbound handshakes (the accept loop), unknown sessions (`link::answer`
closes a stranger for whom no slot is free, and a link gives its slot back
when it ends) and dials (`link::dial` waits for a slot) with semaphores,
and every deadline of section 4: handshake, frame write, frame read,
idle, the unknown-session limits and the age limit. An outbound session
never takes a slot for strangers: a dial sends message 3, and makes a
session, only to a peer that stands for a contact (`PROTOCOL.md` section
4.4).
An accept loop survives listener errors such as too many open files: it
pauses for `ACCEPT_BACKOFF` and goes on while the service is published.
Phase 4 adds the policies that need contacts: closing the oldest silent
stranger for a new one and the contact session budget, per identity and
for the process. Phase 7 adds the rates of contacts (section 6).
Closing the oldest handshake for a new one is not implemented yet, and
the rates of streams and strangers are not enforced: a stream that finds
the handshake budget full is closed and the accept loop pauses for
`ACCEPT_BACKOFF`. The
session layer enforces what belongs to one stream: the message sizes, the
handshake timeout, the frame ceiling of the state, and the session limits
of section 3.

An attacker who also holds a contact card can complete handshakes with
throwaway identities, compete with real strangers for the budget for
strangers, and fill the share of the pending-request queue that requests
without a capability have (section 5). Real requests without a
capability are then dropped until the user handles the queue; requests
that carry an invitation have shares of their own. Sessions with
identities the user holds as accepted or requested are counted
separately and are not affected. The remedy is to rotate the
endpoint, which invalidates the leaked card.

A contact can fill its conversation, `MAX_CONVERSATION_STORE_LEN`, and
several contacts the whole message store. Messages that do not fit are
not taken in and not acknowledged: their senders keep them and try again
later, and the user is told (`Event::MessageRefused`). Room is made only
by the user deleting conversations; nothing stored is dropped by itself.
Since Phase 7 the rates of section 6 bound how fast a contact does so.

Every message taken in rewrites the whole message store (`STORAGE.md`
section 5), up to `MAX_MESSAGE_STORE_FILE_LEN`, 16 MiB, encrypted, and is
acknowledged only once that write is done; a session takes its messages
in one at a time. One contact can so cause one write of the store per
message: after a burst of 60, at most 5 a second at `CHAT_MESSAGE_RATE`,
about 80 MiB of writes a second with a full store, until its
conversation is full. A message takes at least 36 bytes of it
(`ENTRY_OVERHEAD_LEN` and one byte of text), so that is at most about
116 000 writes, about 1.8 TiB with a full store, over six and a half
hours at the rate; what it sends past that is refused and costs no
write. Writes do not add up across contacts: a change made while a write
is under way is carried by the next one, so however many contacts send,
the store is written back to back at most, as fast as the disk takes
it. An EndpointUpdate that brings a newer card writes the vault the same
way, within `CONTROL_MESSAGE_RATE`. The rates bound how fast a contact
writes; they do not make a write cheaper. A store that writes only what
changed (an append-only log, `DESIGN_QUESTIONS.md` 12.3) would, and is a
redesign of the store left for later (P7-9). Blocking the contact stops
it at once.

Whoever copied a contact's identity key, without its active transport key,
can make sessions that present new keys. Each is a pending-key session,
counted with the strangers, so it competes for `MAX_UNKNOWN_SESSIONS`
and not for the room of the contact, and each replaces at most the one
pending card held for that contact.

## 13. File transfers (Phase 8)

The limits of file transfer between accepted contacts
(`adr/0013-file-transfer.md`, `adr/0014-transfer-staging.md`). All are
local except `FILE_WINDOW_CHUNKS` and the chunk size, which both ends of a
session have to agree on. Every limit is checked before anything changes:
before a record is made, a blob is created or a byte is staged. A declared
size never causes an allocation; disk is reserved only after the user
accepted, and within the quotas below.

### 13.1 Memory

| Item | Bound |
| --- | --- |
| A chunk in flight | one frame: `MAX_FILE_CHUNK_LEN`, 64490 bytes, in a frame of at most `MAX_FRAME_CIPHERTEXT_LEN` |
| Staging a file | one segment buffer at a time, 64490 bytes plus its tag, and the hash state; neither side holds a whole file |
| Files staged or saved at a time | `MAX_STAGING_JOBS`, 1 |
| Ended identifiers per session | `MAX_ENDED_TRANSFER_IDS`, 64 |
| Ephemeral installation | the file is staged in memory, in the same format: the declared sizes of both directions together are at most `MAX_EPHEMERAL_STAGED_BYTES`, 256 MiB; each segment buffer is allocated, fallibly, when it is written |

The 256 MiB of an ephemeral installation is memory the peer can fill, but
only with what the user accepted or offered, up to that sum. It is not
counted in the total of section 11.

### 13.2 Disk and records

| Constant | Value | What it bounds |
| --- | --- | --- |
| `MAX_TRANSFER_SIZE` | 256 MiB | One file, sent or accepted. An offer above it is answered with FileReject and leaves no record. |
| `MAX_STAGED_OUTGOING_BYTES` | 1 GiB | Declared sizes of outgoing transfers not finished, per installation. |
| `MAX_STAGED_INCOMING_BYTES` | 1 GiB | Declared sizes of incoming transfers accepted, per installation. A complete one counts until the user saves or removes it. |
| `FILE_FREE_SPACE_MARGIN` | 64 MiB | Space every reservation leaves free: at least what the stores need to be written once more (16 + 1 + 16 MiB). |
| `MAX_TRANSFER_RECORDS_PER_CONTACT` | 16 | Records of one contact, ended ones included. |
| `MAX_TRANSFER_RECORDS` | 256 | Records of the installation. |
| `MAX_TRANSFER_STORE_FILE_LEN` | 1 MiB | The transfer store file. |
| `MAX_PENDING_FILE_OFFERS_PER_CONTACT` | 4 | Incoming offers waiting for the user, per contact. |
| `MAX_PENDING_FILE_OFFERS` | 16 | The same, per local identity. |
| `MAX_ACTIVE_TRANSFERS_PER_CONTACT` | 2 | Incoming transfers accepted and not complete, per contact. |
| `MAX_ACTIVE_TRANSFERS` | 4 | The same, per local identity. |
| `MAX_WAITING_OFFERS_PER_CONTACT` | 4 | Outgoing transfers not yet accepted, cancellations not yet told included, per contact. |
| `MAX_WAITING_OFFERS` | 16 | The same, per local identity. |

Enforcement is in `crate::transfers`, which holds the state of the
installation under one lock and checks each limit before it changes a
record. What an offer that is over a limit gets:

- Over `MAX_TRANSFER_SIZE` or `MAX_PENDING_FILE_OFFERS_PER_CONTACT`:
  FileReject, no record. The same offer gets the same answer every time.
- Over `MAX_PENDING_FILE_OFFERS`, `MAX_TRANSFER_RECORDS_PER_CONTACT` or
  `MAX_TRANSFER_RECORDS`: no answer and no record. The offerer repeats the
  offer on a later session.
- Acceptance by the user needs room in `MAX_STAGED_INCOMING_BYTES`, a free
  place among the active transfers, and the whole blob reserved on disk with
  `FILE_FREE_SPACE_MARGIN` left over. If any is missing the offer stays
  pending and the interface says why. An accepted transfer is never removed
  to make room for another.
- The offerer never refuses an acceptance because of its own limits; the
  limits of active transfers bind the side that accepts.

To make room among records, the store removes, oldest first: an ended
outgoing record; then an ended incoming record that ended more than
`TRANSFER_STALL_LIMIT` plus a day ago; then an ended incoming record
whose final answer the peer is known to have read. A record of a transfer
that is not ended, and one that says the peer is not known to have read a
decision or a cancellation, is never removed for this. If nothing can go,
an offer leaves no record and a file the user would send is refused with a
message that the list of transfers is full.

A blob is reserved at its full length when it is created, so the disk
needed by a transfer is known when it is accepted. A blob is
`32 + k * (40 + L)` bytes for k segments of L = 64490 bytes. The staged
copy is a second copy of the file, besides the place where it is saved.

### 13.3 Time

| Constant | Value | What it bounds |
| --- | --- | --- |
| `FILE_OFFER_TIMEOUT` | 24 hours | An incoming offer the user has not decided. Then the record is expired and the next offer of it gets FileReject. |
| `TRANSFER_STALL_LIMIT` | 7 days | A transfer that made no progress for this long fails, on either side. |
| `ENDED_TRANSFER_RETENTION` | 14 days | How long the record of an ended incoming transfer is kept, so that an offer repeated after a lost answer finds it. Above `TRANSFER_STALL_LIMIT`, so a receiver holds the record of any transfer an offerer may still offer. |
| `TRANSFER_PROGRESS_BYTES` | 4 MiB | Growth of a durable offset that counts as progress. A contact can reset the delays of the scheduler at most `size / TRANSFER_PROGRESS_BYTES` times per transfer, 64 for the largest file. |
| `TRANSFER_PROGRESS_RECORD_INTERVAL` | 1 hour | At most one write of the transfer store per transfer for its stall deadline. |
| `TRANSFER_HOLD` | 10 min | How long a receiver holds a transfer after a write of its blob failed for a full disk or a file held by another process. During the hold its offers are answered with nothing and it makes no dial. The user may resume it sooner. |
| `FILE_PROGRESS_CHUNKS` | 2 | Chunks made durable between two acknowledgements of a receiver. |
| `FILE_PROGRESS_INTERVAL` | 30 s | Longest time between two acknowledgements while data arrives; well below `IDLE_TIMEOUT`. |
| `TRANSFER_UNOFFERED_SESSIONS` | 3 | Sessions in a row on which the offerer did not offer an accepted transfer, after which the receiver fails it. |
| `TRANSFER_GRACE` | 60 s | Reading time a session must have spent as the only one of the contact to count for the rule above. Below `SESSION_LINGER`. |
| `MIN_TRANSFER_THROUGHPUT` | 10 KB/s | The floor the window is sized for. |

A session is not idle while a transfer on it made progress (a chunk read
or written, an offset acknowledged) within the last `SESSION_LINGER`.
Discarded messages are not activity.

### 13.4 Sessions and dials

The reconnect schedule of section 7 is unchanged: `RECONNECT_DELAY_INITIAL`
10 s, doubled after each failure up to `RECONNECT_DELAY_MAX` 30 min, each
delay multiplied by a random factor of 50 percent either way
(`RECONNECT_JITTER_PERCENT`), and reset after a session stayed up for
`RECONNECT_RESET_AFTER`. What Phase 8 adds is when a contact is dialed.

- The scheduler of a contact runs while a transfer needs a session. On the
  offerer: an offer not yet read, a transfer being sent, a cancellation not
  yet read. On the receiver: a transfer accepted and not complete, and a
  cancellation of its own not yet read.
- An offer waiting for the user, a rejection or an expiry not yet read, a
  final answer not yet read, and a transfer held after a failed write make
  no dial on either side. Cancelling or failing a transfer removes its
  claim.
- A dial made for transfers alone resets the delays only if the session
  brought progress to one of them, as `TRANSFER_PROGRESS_BYTES` defines
  it. Otherwise the delays keep growing, as after an acknowledgement
  timeout. A contact that accepts and then never sends, sends one chunk
  per session, or never answers, cannot make this side dial it every few
  minutes, which would tell it when this side is online. The test
  `a_dial_for_a_transfer_alone_that_brings_no_progress_is_repeated_with_growing_delays`
  covers this.
- An offer repeated, or an offset acknowledged again, is not progress.
- The offerer offers a transfer at most once per session, only on a
  session that is confirmed, is its preferred one for the contact and is
  the only one it has with the contact.
- A session ends at `MAX_SESSION_LIFETIME` whatever its transfers do.
  Transfers go on over the next session, from the offset the receiver
  holds.

### 13.5 Fairness

| Constant | Value | Notes |
| --- | --- | --- |
| `FILE_WINDOW_CHUNKS` | 4 | Chunks the offerer may have written beyond the last offset the receiver acknowledged on the session: about 252 KiB. With the window full it writes no chunk until a FileAccept moves the offset. |

The window is an obligation of the sender. The receiver does not end a
session for a chunk beyond it, because it cannot know which of its
acknowledgements the sender had read when it wrote the chunk: a check
against the last FileAccept it wrote would catch some breaches and prove
none. What the receiver checks bounds what a sender that ignores the window
gets:

- the length and the place of every chunk;
- nothing beyond the declared size: a chunk beyond it is a violation and
  ends the session;
- nothing staged beyond the blob reserved at acceptance;
- each chunk written before the next frame is read, so that the receiver's
  own writes go out between chunks.

A sender that ignores the window makes the receiver stage no more than the
declared size, which the user accepted and which is reserved already. It
gains speed in its own direction of the session and nothing else. A
sender that resumes from another offset than the one it was given puts
chunks of the right length in the wrong place; this shows at FileComplete
as a digest that does not match. The cost of that fault is bandwidth,
staging and one pass of hashing over the declared size, never a wrong file.
The test
`a_sender_that_ignores_the_window_stages_no_more_than_the_file_and_holds_up_no_chat`
covers the first case.

Order of writes in a session:

- Replies to what was read come first (Pong, MessageAck, file answers,
  FileAccept progress), then the head of the chat queue, then decisions
  and cancellations of the user, then offers at their pace, then at most
  one FileChunk.
- Before it writes a chunk the session task takes every complete frame
  that has arrived without waiting for one that has not. With transfers in
  both directions neither side stops reading.
- A chat or control message waiting to be written goes out after at most
  one chunk, and waits behind at most the window. At `MIN_TRANSFER_THROUGHPUT`
  a full window takes about 26 s, below half of `MESSAGE_ACK_TIMEOUT`. The
  test `a_chat_message_waiting_while_chunks_go_out_is_written_after_at_most_one_chunk`
  covers it.
- One transfer per contact sends at a time, the oldest accepted first.
- The receiver writes each chunk before it reads the next frame, and
  reads no frame while it verifies a file.

Between contacts:

- No bucket is shared between contacts. A contact that is at its own limit
  is answered as section 13.2 says; its limit is not drawn from another
  contact's.
- A contact that floods chunks is bounded by the declared size of the
  transfer the user accepted, and the receiver reads a chunk only after it
  has written the one before. It holds up neither the chat nor the file of
  another contact. The test
  `a_contact_flooding_its_chunks_holds_up_neither_the_chat_nor_the_file_of_another`
  covers it.
- What a contact can make the receiver decrypt and hash is at most what it
  declared and the receiver accepted, once per transfer at completion, plus
  one pass over an accepted blob after each start of the installation. A
  digest mismatch fails the transfer for good.
- Offers above the pending limit of a contact are answered with FileReject
  and leave no record
  (`offers_above_the_pending_limit_of_a_contact_are_rejected_without_a_record`).
- Through lost sessions, repeated offers and a full disk, a receiver keeps
  one record and at most one staged file for what it accepted, never more
  bytes than the size, and neither writes nor dials more than a few times
  an hour while nothing can be written or no sender can be reached
  (`a_receiver_stays_bounded_through_lost_sessions_repeated_offers_and_a_full_disk`).

### 13.6 What the limits of transfers do not prevent

- Contacts can fill the pending offers of the identity
  (`MAX_PENDING_FILE_OFFERS`, 16) or the records of the installation, and so
  keep other offers from being taken in; those are answered with nothing
  and repeated by their senders. Blocking a contact cancels its transfers
  and removes their staged data.
- The user can be made to hold up to `MAX_STAGED_INCOMING_BYTES`, 1 GiB of
  disk, but only by accepting offers.
- A contact that accepts and then does not read holds an outgoing transfer
  and its staged copy until `TRANSFER_STALL_LIMIT`, 7 days, or until the
  user cancels it.
- Below `MIN_TRANSFER_THROUGHPUT` a chat acknowledgement can time out and
  end the session, which the transfer survives. Below about 25 KB/s a
  duplicate probe can wait behind the window past its timeout and close the
  session it probes, after which the transfer goes on over the other one.
