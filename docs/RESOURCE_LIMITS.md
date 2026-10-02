# Resource limits

Every limit below is a constant in `crates/monolith-protocol/src/limits.rs`.
That module is the only place where such numbers are defined. This document
explains them; the two are changed together.

Rule for all sizes: compare the declared size with the limit first, allocate
second. No allocation is sized by peer input that has not passed that
comparison.

The values are starting points chosen by analysis, not measurement. Phase 9
benchmarks may move them. Lowering a limit is a compatible change only for
limits marked "local"; the others are part of the wire protocol. The cost
of the handshake in section 11.1 is measured.

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
| `MAX_FILE_SIZE` | 4 GiB | - | Largest size accepted in an offer. The field is 64 bits wide. With no resume and bounded session lifetime, larger files are out of scope for version 1. |
| `DEFAULT_MAX_FILE_SIZE` | 1 GiB | - | Local policy, configurable up to `MAX_FILE_SIZE`. |

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

## 4. Timeouts (local)

| Constant | Value | Purpose |
| --- | --- | --- |
| `CONNECT_TIMEOUT` | 120 s | Outbound stream through Tor. Onion connections need a descriptor fetch, an introduction and a rendezvous. |
| `HANDSHAKE_TIMEOUT` | 30 s | From stream open to an authenticated state. Covers three messages over an established circuit. This is the slowloris bound for unauthenticated streams: a handshake message that arrives later is refused, and a peer that sends a valid first message and then nothing holds its pending handshake for this long. |
| `UNKNOWN_SESSION_TIMEOUT` | 20 s | Longest time in `AuthenticatedUnknown`, measured from authentication. Then the session is closed with Close. |
| `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | 10 s | A peer that is not a contact for the session must send its first message within this time, measured from authentication. Then the session is closed with Close. |
| `DUPLICATE_PROBE_TIMEOUT` | 10 s | Liveness probe of an older session during duplicate resolution. |
| `PING_INTERVAL_MIN` / `MAX` | 90 s / 150 s | Each interval is drawn uniformly from this range. |
| `PONG_TIMEOUT` | 60 s | |
| `IDLE_TIMEOUT` | 240 s | No complete frame received, measured from the last complete frame or, before the first, from authentication. Bytes of a frame that is not complete do not count. The session ends without a Close. |
| `FRAME_READ_TIMEOUT` | 60 s | From the first byte of a frame's length prefix to its last byte, an absolute deadline: later bytes of the same frame do not move it. The session ends without a Close. |
| `FRAME_WRITE_TIMEOUT` | 60 s | One frame write. |
| `FILE_OFFER_TIMEOUT` | 10 min | An unanswered file offer is dropped. |
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
idle limit, the age limit of section 3, and while the session is in
`AuthenticatedUnknown` the two unknown-session limits. `link::Link` waits
for that moment alongside the stream and a withdrawal, so each deadline
fires while bytes are awaited and does not depend on a frame completing.
The frame and idle deadlines end the session without a Close; the age and
unknown-session deadlines end it with one, as the local side ends a
session on purpose. Whatever ends a link, it gives back its slot of
`MAX_UNKNOWN_SESSIONS` at once.

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
| `MAX_ACTIVE_INVITATIONS` | 16 | Size of the active set (PROTOCOL.md 12.3). Provisional, see below. |
| `MAX_UI_EVENTS` | 256 | Coalesced, see section 8. |
| `MAX_PENDING_FILE_OFFERS_PER_CONTACT` | 4 | |
| `MAX_PENDING_FILE_OFFERS` | 16 | |
| `MAX_ACTIVE_TRANSFERS_PER_CONTACT` | 2 | |
| `MAX_ACTIVE_TRANSFERS` | 4 | |
| `MAX_ENDED_TRANSFER_IDS` | 64 | Per session; lets messages that crossed an abort be discarded. |
| `FILE_FREE_SPACE_MARGIN` | 64 MiB | Free space required beyond the declared size. |
| `MAX_QUEUED_MESSAGES_PER_CONTACT` | 256 | Outbound, not yet acknowledged. |
| `MAX_QUEUED_MESSAGE_BYTES` | 64 MiB | Outbound, all contacts. |
| `MESSAGE_DEDUP_WINDOW` | 1024 | Received message identifiers kept per contact. |

Tor applies one limit of its own in front of these: `MaxStreams=8` with
`MaxStreamsCloseCircuit` on every `ADD_ONION` (`TOR_CONTROL_SURFACE.md`). It
counts streams on one rendezvous circuit, not streams to the service as a
whole, and a circuit that goes over it is closed. It is defense in depth
only; the budgets above decide what Monolith accepts. The value is
provisional and is reviewed in Phase 4 with the real connection model and
measurements (`DESIGN_QUESTIONS.md` T3-6).

When the handshake budget is full, the oldest unauthenticated stream is
closed to make room for a new one. A flood therefore churns the 16 slots
instead of pinning them, and a connection that completes its handshake
quickly gets through unless the flood rate is high enough to evict it first.
Inbound floods do not affect outbound dials, which have their own budget, so
two contacts can still reach each other as long as one of the two Onion
Services is reachable.

When the budget for strangers is full and another one authenticates, the
oldest that has not yet sent its message is closed.

When the contact session budget is full, no further contact session is
accepted or dialed until one ends. With more than 256 contacts online at the
same time, some stay unreachable. The value is a starting point to be
measured, not a claim that 256 is enough for 1000 contacts.

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
is revoked; nothing is revoked automatically. 16 is the value of Phase 0.
It has not been checked against how many cards users keep in circulation,
and is confirmed or changed with the contact store in Phase 4.

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

No values are chosen here. Phase 4 sets them with the contact store and
the resource tuning, and section 11 is then extended from one identity to
the number of identities a process allows. Phase 3 has one set of
`Budgets` per process; nothing in it prevents a set per identity.

## 6. Rates (local)

Token buckets. "Burst" is the capacity, "per minute" the refill.

| Constant | Burst | Per minute | Scope |
| --- | --- | --- | --- |
| `INBOUND_CONNECTION_RATE` | 32 | 120 | All inbound streams. |
| `UNKNOWN_SESSION_RATE` | 8 | 12 | All unknown identities together. |
| `CHAT_MESSAGE_RATE` | 60 | 300 | Per contact. |
| `CONTROL_MESSAGE_RATE` | 120 | 600 | Per contact. Ping, Pong, MessageAck, EndpointUpdate, file control messages, and ContactRequest or ContactAccept received after confirmation. |
| `FILE_OFFER_RATE` | 4 | 6 | Per contact. |
| `PROFILE_UPDATE_RATE` | 8 | 8 | Per contact. Not below the session rate, because a profile is sent once per session. |
| `CONTACT_SESSION_RATE` | 6 | 6 | New authenticated sessions per identity. |

FileChunk is not rate limited by count. It is limited by the declared size
of an accepted transfer and by backpressure: Monolith stops reading from a
session while the disk writer or the UI queue is behind.

Rates for unknown peers are global because neither a Tor circuit nor a fresh
identity key costs an attacker anything. A stream over the inbound rate is
closed right after accept. A session opened above `CONTACT_SESSION_RATE` is
closed.

The per-contact message rates are enforced by waiting, not by
disconnecting: when a bucket is empty, Monolith stops reading from that
session until it has a token again. A conforming sender therefore does not
need to know the rates, and a sender that resends a full queue after a
reconnect is slowed down, not cut off.

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
delay is multiplied by a uniform random factor in [0.5, 1.5]. The delay is
reset after a session has stayed authenticated for 60 s.

With 1000 offline contacts at the cap, a dial comes due about every 1.8 s
on average. Only 4 run at once. A dial that is due waits for a free slot in
order of due time, so when dials are slow (the timeout is 120 s) the retry
interval stretches; the number of concurrent attempts does not grow.
Because both sides dial, a contact that comes online reaches us promptly
even when our own retry timer is long.

## 8. UI event queue

The core sends events to a front end through one bounded queue of
`MAX_UI_EVENTS` entries. Events that describe state (peer online, request
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

## 11. Worst-case budget

Memory that a peer can influence, with every budget full:

| Item | Bound |
| --- | --- |
| Contact sessions: 256 x (read buffer + write buffer, one frame each) | about 32 MiB |
| Pending handshakes: 16 x about 3 KiB (section 11.1) | about 48 KiB |
| Unconfirmed sessions: one frame of one block each, plus about 2 KiB of session state | under 1 MiB |
| Outbound message queue | 64 MiB |
| UI event queue: 256 x 16.4 KiB | about 4.1 MiB |
| Duplicate windows: 1000 contacts x 16 KiB | about 15.6 MiB |
| Active transfers: 4 x (one chunk + hash state) | about 0.3 MiB |
| Pending requests, file offers | under 0.1 MiB |
| Contacts with profiles, block and declined lists | about 2 MiB |
| Total | about 119 MiB |

The outbound queue is filled by the local user, not by peers. Without it the
peer-driven ceiling is about 55 MiB. Full-size session buffers are allocated
when a session is confirmed as a contact session and freed when it closes,
and a duplicate window exists only for a contact that has sent a message,
so the typical footprint is far lower.

Duplicate windows have to outlive sessions to be of any use, which is why
they are counted per contact and not per session.

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
  `DEFAULT_MAX_FILE_SIZE` each are in progress. The vault is capped at
  16 MiB. The history store quota is defined with the history store
  (ADR 0005, open item).
- CPU: an inbound handshake costs three X25519 operations, two fixed-base
  multiplications and the validation of one contact card, about 0.3 ms
  (section 11.1). At the inbound rate limit that is a
  fraction of a percent of one core.

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
  operation per stream, 0.06 ms, and gets no protocol response. At
  `INBOUND_CONNECTION_RATE`, 120 per minute, that is under 10 ms of
  processor time per minute.
- A caller with the contact card can make the responder do the full
  handshake, about 0.3 ms, and can hold a pending handshake of 2.7 KiB for
  30 seconds. `MAX_INBOUND_HANDSHAKES` bounds the second at 16 x 2.7 KiB.
- The invitation capability is looked at only inside a ContactRequest,
  after a complete handshake. A holder of a card who tries capabilities
  pays a full handshake and one request per attempt, and is bounded by
  `UNKNOWN_SESSION_RATE`, 12 per minute, and by the budget for unknown
  sessions. Every attempt is answered with the same Close.
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

An attacker who knows the Onion Service address can keep the inbound rate
limit exhausted. Legitimate inbound connections then fail until the flood
stops. Tor's proof-of-work defense, where it is active, and the cap on
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
pauses for `ACCEPT_BACKOFF` and goes on while the service is published. The policies that need contacts and
rates (closing the oldest handshake, the rate buckets of section 6, the
contact session budget) come with the application core of Phase 4. The
session layer enforces what belongs to one stream: the message sizes, the
handshake timeout, the frame ceiling of the state, and the session limits
of section 3.

An attacker who also holds a contact card can complete handshakes with
throwaway identities and keep `UNKNOWN_SESSION_RATE` exhausted. Contact
requests from real strangers are then refused for as long as it lasts.
Sessions with identities the user holds as accepted or requested are
counted separately and are not affected. The remedy is to rotate the
endpoint, which invalidates the leaked card.

Whoever copied a contact's identity key, without its active transport key,
can make sessions that present new keys. Each is a pending-key session,
counted with the strangers, so it competes for `MAX_UNKNOWN_SESSIONS`
and not for the room of the contact, and each replaces at most the one
pending card held for that contact.
