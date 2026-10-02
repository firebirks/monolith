# Test plan

Status: the protocol core and the session layer have tests; everything
else is a plan.

What exists after Phase 1: unit tests in `monolith-identity` and
`monolith-protocol` for keys, signatures, fingerprints, base32, the field
codec, text rules, contact cards, message bodies, frames, session logic and
the duplicate rule; property tests in `tests/properties.rs` of both crates;
eight fuzz targets under `fuzz/`. Of the identifiers below, these are
covered at the level the protocol core allows: T-FRAME-1 to 6, T-FIELD,
T-KEY, T-CARD, T-TEXT, T-FILE-NAME, T-PROTO-STATE, T-ENDPOINT-1,
T-CONTACT-2, 4 and 5 (with two sessions wired to each other), T-DUP-1 to 3
and 7, T-ORACLE-1, 3, 4 and 5, and the first half of T-ORACLE-7.
T-ORACLE-2 holds by construction.

What Phase 2 added: tests in `monolith-session` for the resolver, the
handshake and the encrypted session (section 3a); known-answer tests that
reproduce the vectors of PROTOCOL.md section 16.1; a harness of two
sessions connected in memory, with fixed keys; property tests over real
handshakes; three more fuzz targets. In the protocol core: the transport
key, the contact card with the transport key, and the stale-card rule.
Covered now: T-XKEY, T-VEC, T-HS, T-HS-PIN, T-BIND, T-STALE, T-FRAME-AUTH,
T-LIMIT, T-SEND, T-RNG-1 and T-RNG-2 for the handshake, T-ORACLE-8 for
bytes, and the handshake cases of T-MAL.

What Phase 3 added: the ServiceID codec in `monolith-identity`; in
`monolith-tor`, unit tests of the SOCKS client, the control reply parser,
the commands, SAFECOOKIE (against values computed independently) and the
endpoint configuration, and tests of `SystemTorBackend` against scripted
SOCKS and control servers on loopback, hostile ones included
(`tests/system.rs`); in `monolith-core`, Tor streams carrying the Phase 2
session on `MockTorBackend`, with the handshake deadline and the accept
loop's budget (`tests/link.rs`); two more fuzz targets; a Phase 3
mutation list; the fail-closed network test (`tests/network/`); and the
private Tor network test (`tests/tor-network/`), which is written but not
yet run. Covered now: T-SOCKS, T-CTRL, T-NET-1.

The rest need the core, Tor or storage, and are not written yet.

## 1. Principles

- Hostile input is the normal case. Every decoder is tested mostly with
  input it must reject.
- A security-sensitive change arrives with its tests (ARCHITECTURE.md
  section 9).
- Protocol code is sans-IO, so the same decoder runs under unit tests,
  property tests and fuzzers without a network.
- CI never uses the public Tor network.
- Tests that need reduced limits get them from a constructor that exists
  only in test builds of the session crate, not by changing `limits.rs`.
  A product build has the limits of the protocol and no way to set others.
- The specification is tested, not only the library: expected bytes come
  from an implementation that shares no code with Monolith, and a test
  compares the constants in the test code with the text of PROTOCOL.md.
- Tests that need a handshake to run the same way twice fix the ephemeral
  keys. That is possible only inside the tests of the session crate and in
  fuzz builds.

## 2. Levels and tools

| Level | Tool | Where it runs |
| --- | --- | --- |
| Unit | `cargo test` | CI |
| Property | `proptest` | CI |
| Fuzz | `cargo fuzz` (libFuzzer), nightly toolchain | scheduled, and locally |
| Integration | `MockTorBackend`, scripted SOCKS and control servers | CI |
| Private Tor network | Chutney with an onion service network | local, opt-in |
| Platform | Tails and Whonix virtual machines | manual, per release |
| Public Tor | two nodes over the real network | manual |

## 3. Unit tests

Frames (T-FRAME)

1. Length below the minimum is rejected before any read.
2. Length above the maximum is rejected before any read.
3. Length not congruent to 16 modulo 1024 is rejected.
4. In `AuthenticatedUnknown`, any length other than 1040 is rejected.
   Before that state every length is rejected: no frame exists before the
   peer is authenticated.
5. Padding longer than necessary, or non-zero, is rejected.
6. `body_length` larger than the plaintext is rejected.

Fields (T-FIELD): for every variable field, length at maximum is accepted
and maximum plus one is rejected; for every fixed-size message, body length
off by one in each direction is rejected; trailing bytes are rejected;
presence bytes other than 0 and 1 are rejected.

State machine (T-PROTO-STATE): every message type in every state; no
message triggers a dial; no message from an unknown peer changes the
contact store.

Identity (T-ID)

1. An endpoint that does not prove the pinned identity, at handshake
   message 2, is a hard failure and raises the mismatch event. Covered in
   the session layer (T-HS-PIN) and again in the session logic: an outbound
   session ends when it is told of any identity but the dialed one, or of
   the local one.
2. No code path replaces a pinned key.
3. Two contacts with identical display names remain distinct.

Keys and signatures (T-KEY): a malformed compressed point; each of the
eight small-order points; the identity element, which is torsion-free and
must still be rejected; a valid key plus each small-order point, which is
not of small order and must still be rejected; generated keys; every
encoding with y at or above the field prime; all of it for identity keys
and for onion service keys; a signature that the plain verification
equation accepts and strict verification rejects (PROTOCOL.md 16.1).

Transport keys (T-XKEY): the five points of small order; every value from
the field prime upwards; every key with the top bit set; the comparison
with the prime at every byte; the Montgomery form of an Ed25519 key
against values computed outside the code; a property test of validity
against the rule stated with integers.

Contact cards (T-CARD): valid cards of both sizes; every single-bit flip is
rejected; reserved flag bits; epoch zero; wrong version; trailing bytes;
invalid curve points; small-order keys; a validly signed card whose
endpoint is not a valid key, or is the identity key; a validly signed card
whose transport key is not a valid key, or is the identity key or an
endpoint key in Montgomery form; the order of those checks; text form in
lower and mixed case, with whitespace, with a wrong prefix, with non-zero
trailing bits, over length.

Card epochs (T-ENDPOINT)

1. Greater epoch from the pinned identity is accepted as pending; the same
   epoch with the same transport key and endpoint is a no-op; the same
   epoch with another transport key or endpoint is reported as an anomaly;
   a lower epoch is ignored; a card signed by another key is rejected; the
   pinned epoch never decreases.

Text (T-TEXT): invalid UTF-8; each forbidden code point class per field;
bidirectional controls in names and filenames; zero-width characters and
characters that draw as nothing; maximum scalar count for names; names
in any normalization form are accepted and kept byte for byte.

Filenames (T-FILE-NAME): separators, `.` and `..`, control characters,
drive letters, UNC prefixes, reserved device names (with port numbers 0 to
9 and superscript digits, and the console names), leading and trailing
dots and spaces, names that sanitize to nothing, and the length of the
suggested name.

File transfer (T-FILE)

1. A chunk for a transfer that was not accepted closes the session.
2. Nothing is written before the user accepts.
3. Wrong chunk size, more bytes than declared, completion before all bytes,
   digest mismatch, duplicate transfer identifier.
4. Existing destination file is not overwritten.
5. Messages that cross an abort, a reject or an offer timeout are
   discarded. A file message for an identifier never seen on the session
   is a violation.

Contacts (T-CONTACT)

1. A flood of requests leaves the contact store byte-identical and the
   pending queue at its bound.
2. Crossing requests produce one accepted contact on each side.
3. Deleting a contact and receiving a request with the same name yields an
   unknown identity.
4. Confirmation: when one side holds the other as accepted and the other
   side does not (deleted, restored from backup, never accepted), the
   session is not confirmed and no contact-only message is sent in either
   direction.
5. A side whose record says "requested" becomes "accepted" on ContactAccept
   and the session is confirmed without a reconnect.

Blocking (T-BLOCK)

1. A blocked identity produces no event and no stored state, and receives
   exactly what a stranger with a bad invitation receives.
2. Blocking an accepted contact ends its session; its later sessions are
   never confirmed.

Invitations (T-INV)

PROTOCOL.md sections 12.2 and 12.3. The active set and revocation are part
of the contact store, so these tests are mandatory in Phase 4 and run
against it; only the card part of T-INV-1 exists today (`card::tests`:
cards that differ only in their capability are each valid, have their own
signature and do not conflict).

1. One identity with cards that carry capabilities A and B: both cards are
   valid, both capabilities are in the active set, and a request with
   either one is queued.
2. After A is revoked, a request with A is dropped.
3. After A is revoked, a request with B is still queued.
4. B is not used up: requests with B from several identities are queued,
   up to `MAX_PENDING_REQUESTS_PER_INVITATION`, and B is still valid after
   they were handled.
5. A request with a revoked capability gets what a request with a valid
   one gets: the same messages, bytes and close condition (with T-ORACLE-1
   and T-CONFIRM-1).
6. A request with a capability that was never issued, or that belongs to
   the card of another identity, gets the same.
7. A contact that was accepted from a request with A stays accepted after
   A is revoked: its record, its pinned card and an open session are
   unchanged, and its next session is confirmed.
8. With `MAX_ACTIVE_INVITATIONS` capabilities in the set, creating another
   one fails and no capability is revoked.
9. The local label of a capability appears in no card bytes and in no
   frame.
10. For a peer that is not an accepted contact, importing a card of the
    same identity, transport key, endpoints and epoch with a different
    capability, or with none, replaces the held card, and the next request
    carries the new capability. For an accepted contact the same import
    changes neither the contact nor its sessions.
11. Revoking a capability leaves the pending requests it admitted in the
    queue. If the action that revokes and discards is offered, it removes
    exactly the requests that capability admitted; the record of which
    capability admitted a request appears in no frame.

Duplicate sessions (T-DUP)

1. Simultaneous dial in both directions: both sides keep the same session.
2. One session completes much earlier than the other.
3. Same-direction reconnect while the old session is half-open.
4. Reconnect storm from one contact is rate limited.
5. Unacknowledged messages move to the surviving session exactly once.
6. An unauthenticated stream cannot affect an existing session.
7. The preferred session is half-open: the probe gets no Pong and the newer
   session is kept.

Randomness (T-RNG)

1. A failing CSPRNG source makes key generation fail; there is no fallback.
   Covered for the handshake's key object, where a source that fails
   leaves no key behind, and for a whole handshake, which fails with
   `SessionError::Randomness`. A transport secret key of 32 zero bytes,
   what a buffer that was never filled holds, is refused.
2. The operating system source fills its whole buffer; two generated keys
   differ; two handshakes with fresh randomness differ in every message.

Tor control (T-CTRL)

1. With a recording mock control port, a full run emits only the lines in
   TOR_CONTROL_SURFACE.md. Covered: `system.rs` (status and publication
   send exactly the allowed lines) and `command::tests` (the bytes of
   every command).
2. Oversized lines, too many lines, too many bytes, LF without CR, data
   blocks, events, `510` replies, a closed connection in the middle of a
   reply. Covered: `reply::tests`, `system.rs`, fuzz target
   `tor_control_reply`.
3. SAFECOOKIE: the hashes match an independent computation; a server
   with a wrong hash never receives `AUTHENTICATE`; the configuration, not
   the server, chooses the method; a cookie file that is not a regular
   32-byte file is refused. Covered: `auth::tests`, `system.rs`.
4. Publication: the two `ADD_ONION` forms, a malformed or wrong ServiceID,
   a malformed, missing or unexpected key, non-anonymous mode, `DEL_ONION`
   failure, and control loss, which unpublishes the service before and
   during `accept`. Covered: `control::tests`, `system.rs`.

SOCKS (T-SOCKS)

1. The request carries address type 0x03, the literal `.onion` name and
   port 29170. Covered: `socks::tests`, `system.rs`.
2. Refused method, error replies, malformed, truncated and fragmented
   replies, stalls in each phase, and that nothing after the reply is
   consumed. Covered: `socks::tests`, fuzz target `socks_reply`.

Logging (T-LOG)

1. The hostile-peer suite runs at maximum verbosity; the output contains
   none of the secrets and none of the peer-supplied strings used.

## 3a. Session layer

These are in `monolith-session`. A hostile peer is played with the Noise
library directly, so that it can send what the types of the crate would
never produce.

Known answers (T-VEC): the handshake between the two parties of PROTOCOL.md
16.1 with the ephemeral keys given there produces the three messages, the
handshake hash and the first frames of each side that the document states.
The values the document states are searched for in its text; it gives the
frames as SHA-256 digests. The test also pins the first 34 bytes of the
initiator's first frame, which the document does not list.
X25519 against RFC 7748, the cipher against values computed outside the
code with the Noise nonce, SHA-256 and the HMAC built on it against the
standard vectors.

Handshake (T-HS)

1. An invalid public key: each point of small order and a key with the top
   bit set, as the ephemeral key of message 1 and of message 2, and as the
   transport key in message 3.
2. Malformed messages: constant bytes of each message size; a payload of
   another size in message 3. Truncated and oversized messages cannot be
   presented: a message is taken as an array of its exact size, and the
   reader takes that many bytes from the stream and no more.
3. One bit changed in every byte of every message; a property test over
   arbitrary bits; arbitrary bytes, also behind a valid ephemeral key.
4. Replay: message 1 to a new responder, which answers with a new key;
   message 3 to that responder; message 2 to a new initiator; message 3
   again on the established session.
5. Reflection: message 1 to its sender as message 2; message 2 to a
   responder as message 1; an initiator's message to a responder of the
   same party.
6. Another label, no prologue, another identity in the prologue, the
   identity in front of the label; a responder under another label.
7. A handshake that stalls at each step ends at `HANDSHAKE_TIMEOUT`, and
   not before.
8. Two handshakes with randomness from the operating system complete and
   differ.
9. No long-term key, signature or endpoint appears in a handshake message,
   and message 1 does not depend on who sends it.

Outbound pinning (T-HS-PIN): an endpoint with its own keys cannot read
message 1; a forged message 2 and a genuine message 2 of another handshake
are refused with `IdentityMismatch`; a party cannot dial its own identity.

Identity binding (T-BIND)

1. A card that names another party's transport key: the party that holds
   the key refuses message 1. The same handshake is accepted if the
   responder puts the card's identity into its prologue, which shows that
   the prologue is what stops it.
2. In message 3: a valid card of the key holder is accepted from anybody;
   somebody else's card, a card with the key rewritten, a card of an
   identity that names another key, bytes that are not a card, the front
   of a card with an invitation, and a card of the responder's own
   identity are refused.
3. The local side cannot be built from a card and a transport key that do
   not belong together.

Stale cards (T-STALE): the standing of a peer for every record and every
relation between the card of the session and the newest card held; a
retired transport key against a contact that has received the new card,
before and after its user confirmed it; a dialed card that was superseded
while the dial was in progress; a newer card keeps the contact and is
reported as a pending change.

Frames on a session (T-FRAME-AUTH)

1. Messages of every type cross a confirmed session in both directions;
   frame sizes are those of the specification; any fragmentation of the
   stream gives the same messages.
2. A changed byte, in the ciphertext, the tag or the length prefix, ends
   the session; nothing is delivered from that frame or after it.
3. A dropped, repeated or reordered frame ends the session.
4. A frame of another session, and a frame sent back to its sender, are
   rejected.
5. Nothing is processed after a Close was sent or received; a Close is
   produced once.
6. An application message before confirmation is not sent and, if a peer
   sends one, ends the session.
7. A card of another identity inside a frame that authenticates ends the
   session; the local side does not send one.
8. A frame that authenticates and is malformed inside ends the session.

Session limits (T-LIMIT): with reduced limits, the frame limit, the byte
limit and the age limit each stop sending and leave room for a Close; a
peer that goes past the frame or byte limit ends the session at the
receiver; a Close and the last frames that arrive within the grace after
the age limit are an orderly end, and a frame after the grace is a
violation; a transfer extends the age limit to its own bound and no
further, and not at all if it starts after the limit.

Sending rules (T-SEND): a request is sent once on a session; it carries
the local card and the invitation capability of the card of the peer that
is asked, and no other; nothing is sent on a confirmed session before the
local ContactAccept; Close is not sent like a message.

Key erasure at the end of a session: the cipher keys are gone after a
violation, after a Close in either direction, and after the stream was
reported closed; a session that has to answer with Close keeps them until
that Close has been made.

Two-party harness: two sessions connected in memory carry out what the
session logic tells them. Everything one side writes is recorded, so that
"the peer sees the same thing" is compared byte for byte.

## 4. Property tests

- Encode then decode is the identity for every message type.
- Decode then encode reproduces the input bytes exactly for every accepted
  input (canonical form).
- Arbitrary bytes never decode to a value that violates a field limit.
- Arbitrary sequences of events (messages from the peer, with its own card
  or another one; local close, block, removal; end of the stream) never
  deliver application data on a session that is not confirmed, never move
  a session backwards, and never let the session send what `may_send`
  forbids.
- Arbitrary length prefixes never cause an allocation larger than one
  frame.
- The duplicate-session rule gives the same winner on both sides for any
  pair of identity keys and any interleaving.
- Filename sanitization output never contains a separator and is never
  empty.
- The standing of an inbound peer follows the table of PROTOCOL.md 6.2 for
  every record and every pair of presented and pinned card.
- Any bit changed in any handshake message makes the handshake fail.
- A stream of frames split at arbitrary points delivers the messages that
  were sent, in order.
- A byte changed anywhere in a stream of frames delivers nothing from the
  frame it is in or after it.
- Arbitrary bytes fed to a session never reach the application.
- The two directions of a session do not depend on each other.
- A session never sends more frames or bytes than its limits, whatever is
  sent, and a Close always fits.

Random bytes almost never pass the first check of a decoder, so a property
stated over random bytes mostly tests that check. The properties start from
valid values and damage them: one byte of a valid frame or body, one field
of a valid card replaced by the same field of another valid card, one
symbol of a valid card text. Text properties draw from pools that contain
the characters the rules are about and compare the validator with the rule
written out a second time.

## 5. Fuzzing

Targets in `fuzz/`:

| Target | Input | State |
| --- | --- | --- |
| `base32` | text | exists |
| `contact_card` | binary card | exists |
| `contact_card_text` | text card | exists |
| `message_body` | type selector and body; covers endpoint updates | exists |
| `frame_plaintext` | state selector and plaintext frame | exists |
| `frame_stream` | byte stream split at arbitrary points | exists |
| `text_fields` | bytes, against every text type and the save-name suggestion | exists |
| `session_sequence` | sequence of messages and local events against a session | exists |
| `handshake_responder` | a stream from an initiator, raw or the genuine one with damage, under the records none, blocked, requested and accepted | exists |
| `handshake_initiator` | a stream from a responder, raw or the genuine one with damage | exists |
| `session_frames` | operations on two connected sessions: send, deliver, change, drop, repeat, inject, close, block, remove, stream closed, time passing | exists |
| `tor_control_reply` | bytes from a control endpoint, in pieces; key bytes for `ADD_ONION` | exists |
| `socks_reply` | bytes from a SOCKS proxy after a CONNECT | exists |
| `vault_header` | bytes | with storage |

Seed inputs for every target are in `fuzz/seeds/`. They are generated by
the test `fuzz_seeds` in `monolith-protocol` and, for the three session
targets, by the test `seeds` in `monolith-session`. Both fail when a
committed seed no longer matches what the current code produces.

The session targets run real handshakes with fixed ephemeral keys, so that
an input behaves the same way on every run. The contact card changed in
Phase 2, so the seeds that contain a card were regenerated; the other
Phase 1 seeds changed only where the removed message type shifted a
selector.

The existing targets have had short smoke runs only. Long runs with a kept
corpus are not done; see `fuzz/README.md`.

The property every target enforces:

For arbitrary bytes from a peer, Monolith either rejects them or parses
them into a valid bounded message, without a panic, without running out of
memory, without an allocation larger than one frame, without an unbounded
loop, without an unbounded write, without leaving the designated
directory, and without changing persistent state before authentication and
validation.

Harness rules: an allocation cap well below the default (`-malloc_limit_mb`),
a per-input timeout, and for stateful targets an in-memory store whose
contents are compared before and after for unauthenticated input. A corpus
is kept in the repository once it exists. Crashes become regression tests.

## 6. Hostile peer suite (T-MAL)

Run against a real session over `MockTorBackend`:

- endless input without completing a handshake record or a frame;
- frames at maximum length and at maximum plus one;
- length prefixes of 0, 1, 65535;
- truncated handshake and truncated frames;
- duplicated and reordered frames;
- random bytes as frames;
- messages replayed across sessions;
- invalid UTF-8, large combining sequences, bidirectional controls;
- a flood of contact requests from many identities;
- stalling at every step of authentication;
- a flood of file offers;
- traversal and device names as filenames;
- wrong file size, too many chunks, premature completion;
- stale endpoint epoch;
- a handshake message of another session, a handshake message sent back to
  its sender, a card that does not belong to the key holder;
- an identity mismatch on an outbound session.

The handshake and frame cases of this list are covered without a network
in the session layer (section 3a). The cases that need the core's budgets
and queues come with Phase 4.

Each case asserts the outcome (session closed or input ignored), that no
event reached the front end unless specified, and that budgets held.

Conforming peers under load, in the same harness: a contact that sends
faster than a per-contact rate is slowed and not disconnected; a full
outbound queue resent after a reconnect arrives completely; a profile sent
on each of several quick reconnects does not end the session.

Confirmation tests (T-CONFIRM)

1. An unknown peer's view (bytes received and time to close) is the same
   whether its request was queued, dropped for a bad invitation, dropped as
   a duplicate, dropped because the sender is blocked, or dropped because
   the queue was full, and whether or not contact sessions exist.
2. No message type contains a field for a third party's identity or
   address (checked against the message table).

Observability matrix (T-ORACLE)

The peer classes are those of PROTOCOL.md section 12.1: no record, blocked,
deleted former contact, declined, requested, accepted and verified, accepted
and not verified. "Observable" means what the local side emits toward the
peer and when it closes.

| Id | What is compared | Level |
| --- | --- | --- |
| T-ORACLE-1 | For no record, blocked, deleted and declined: the emitted messages (type, number, order) and the close condition are identical when the peer stays silent, sends a ContactRequest with a valid, an invalid or no invitation, sends ContactAccept, or sends Close | session logic, Phase 1 |
| T-ORACLE-2 | Accepted-verified and accepted-unverified are identical in every emitted message and state transition; the verification mark is not an input of any function that produces protocol output | by construction, Phase 1: `Standing` has one variant for accepted contacts and no function of `Session` takes a verification mark, so there is nothing to compare at run time. Checked again in review when `monolith-core` adds the mark. |
| T-ORACLE-3 | No message type and no field carries a rejection reason; Close has an empty body; there is no "blocked", "former contact" or "not in contact list" response | message registry, Phase 1 |
| T-ORACLE-4 | A protocol violation from any class ends the session with nothing emitted. Includes a ContactRequest with the card of another identity | session logic, Phase 1 |
| T-ORACLE-5 | Profile, EndpointUpdate and application messages are never emitted before confirmation, for any class | session logic, Phase 1 |
| T-ORACLE-6 | With the budget for strangers exhausted, the four non-contact classes are treated identically | core, Phase 4 |
| T-ORACLE-7 | Blocking or deleting an accepted contact during a session: the peer sees Close, then the behavior of the no-record class | first half in the session logic, Phase 1; the later sessions in core, Phase 4 |
| T-ORACLE-8 | On the wire: frame counts and byte counts are equal across the non-contact classes. Response times are recorded and compared only for gross differences | bytes: session layer, Phase 2, where everything one side writes is identical for no record, declined, blocked, and a contact with a stale or conflicting card; over the mock transport and with times: integration, Phase 4 |

The handshake is the same for every class by construction: the responder's
handshake functions take no record of the peer, and the record is first
looked at when the handshake is complete. The class "a contact that
presented a stale or conflicting card" was added in Phase 2 and is part of
T-ORACLE-1 and T-ORACLE-8.

These tests do not claim constant-time behavior. They check that the cases
cannot be told apart by what is sent.

## 7. Concurrency

- Simultaneous connections in both directions.
- Shutdown while connecting, during a handshake, during a transfer.
- Tor disappears: SOCKS refuses, control connection closes.
- The control connection drops and returns; the service is re-created.
- A SOCKS connection stalls forever.
- The message store is unavailable; the vault is locked by another process.
- Cancellation of every kind of task leaves no task behind.

## 8. Resources (T-RES)

1. Memory high-water mark under the hostile suite stays under the computed
   ceiling of RESOURCE_LIMITS.md section 11.
2. Task count stays under its bound under a connection flood.
3. Every channel is bounded: lint, plus a test that fills each queue and
   observes backpressure.
4. File descriptors stay under their bound under a connection flood.
5. Notifications and pending requests stay at their bounds under a request
   flood.

## 9. Tails matrix (T-PLAT-TAILS, manual)

Before any Tor code is written (precondition for Phase 3 and Phase 6), on a
current supported Tails release, without changing Tor's configuration and
with the sandbox left on:

| Id | Check |
| --- | --- |
| T-PLAT-TAILS-PRE-1 | `ADD_ONION` through port 951 with `Sandbox 1`: accepted or refused, with Tor's reply |
| T-PLAT-TAILS-PRE-2 | Trace of what OnionShare sends to publish a service, and through which path |
| T-PLAT-TAILS-PRE-3 | List of integration OnionShare has that a third-party package lacks |
| T-PLAT-TAILS-PRE-4 | The draft Monolith profile loaded and exercised; corrected profile recorded |
| T-PLAT-TAILS-PRE-5 | What an installation that survives reboot requires |
| T-PLAT-TAILS-PRE-6 | For each workaround considered: effect on the sandbox and on Tor's configuration |

The results go into `PLATFORM_TAILS.md` section 3.4.

Per release, on the current Tails:

| Case | Expectation |
| --- | --- |
| Fresh boot, no Persistent Storage | runs ephemeral; `EPHEMERAL SESSION` shown; nothing written |
| Persistent Storage present but locked | as above; creating a persistent identity is refused with an explanation |
| Persistent Storage unlocked | persistent identity can be created under `~/Persistent`; survives reboot |
| Tor not yet bootstrapped | "Tor connecting"; no traffic attempted |
| Tor disconnect and reconnect | service removed and re-created; sessions resume |
| Profile installed | service publishes through port 951 |
| Profile missing | clear error naming the filtered command; no crash |
| `ADD_ONION` under `Sandbox 1` | as established by T-PLAT-TAILS-PRE-1 |
| Two Monolith peers | chat both ways |
| File transfer | offer, accept, complete; reject; abort |
| Restart of the application | ephemeral identity gone; persistent identity back |
| Power off and reboot | no trace outside Persistent Storage |
| Sockets held | only SOCKS 9050, filter 951 and the listener |
| Firewall reject log | no entry caused by Monolith |
| One tor process | yes |

## 10. Whonix matrix (T-PLAT-WHONIX, manual)

Before Phase 7, with one Gateway and two Workstations (W1 runs Monolith, W2
is the attacker), on VirtualBox or KVM, and separately on Qubes-Whonix:

| Id | Check | Expectation |
| --- | --- | --- |
| T-PLAT-WHONIX-ISO-1 | The rule of section 4.5 is present after boot and after a restart of the firewall service | present; the port is not in `EXTERNAL_OPEN_PORTS` |
| T-PLAT-WHONIX-ISO-2 | W2 connects to W1 port 29170 from its own address | no connection |
| T-PLAT-WHONIX-ISO-3 | Same, with the port opened the supported way instead (no source restriction) | connection is accepted by the kernel and closed by Monolith with no byte sent |
| T-PLAT-WHONIX-ISO-4 | W1 listener over IPv6 and on any other interface | not reachable |
| T-PLAT-WHONIX-ISO-5 | Manual firewall reload on W1 | port closed until the service is restarted |
| T-PLAT-WHONIX-ISO-6 | W2, with root, takes the Gateway's address on the shared segment | recorded as found. Expected on non-Qubes: W2 reaches the handshake as an unknown peer. Expected on Qubes: not possible |
| T-PLAT-WHONIX-ISO-7 | With W2 past the network layers and holding W1's contact card | W2 learns that the identity runs on W1 and nothing else; without the card, what it learns is recorded against the session layer in use |
| T-PLAT-WHONIX-ISO-8 | The Monolith onion-grater profile enabled on the Gateway | W2 can create services pointing at its own port 29170 only |

The isolation of the listener is not described as working anywhere in the
documentation until these have been run.

Per release, on the current Whonix, VirtualBox or KVM, and Qubes-Whonix:

| Case | Expectation |
| --- | --- |
| Gateway and Workstation, profile enabled, port opened | service publishes; chat works |
| Profile not enabled | clear error naming the filtered command |
| Firewall port not opened | service publishes but no inbound session; `doctor` reports it |
| Gateway Tor unavailable | "Tor disconnected"; nothing sent |
| Workstation restart | persistent identity back; service re-created |
| Gateway restart | service re-created after reconnect |
| SOCKS isolation | streams of two contacts use different circuits (checked on the Gateway) |
| Unauthorized control commands | `SETCONF`, `GETINFO address` and other commands outside the merged profile are refused by the Gateway |
| Listener exposure | bound to the Workstation's internal IPv4 address only; not on any other interface |
| Second Workstation connects to the listener directly | T-PLAT-WHONIX-ISO-2 and 3 |
| One Tor | no tor process in the Workstation |
| Qubes-Whonix | control address, listener address and firewall drop-in work with dynamic addresses |

## 11. Network privacy (T-NET)

1. In a Linux network namespace in which only Tor's SOCKS and control
   endpoints are reachable: Monolith works. With Tor stopped: every network
   operation fails,
   and a packet capture on the namespace shows no DNS query, no direct TCP
   connection, no HTTP and no other traffic.
2. With the SOCKS endpoint configured but refusing connections, dials fail
   and are retried on schedule; nothing else is attempted.

An idle run produces no traffic other than keepalives on open sessions and
the reconnect schedule.

On Tails this cannot be shown by blocking, because Tails redirects direct
TCP into Tor. The namespace test on a development machine is the proof; the
Tails matrix checks sockets and the firewall log.

## 12. Injection (T-INJ)

Through every peer-supplied text field (display name, profile text,
introduction, chat text, filename): newline, the field separators of every
storage record, SQL metacharacters, JSON and TOML syntax, path separators
and traversal, terminal escape sequences, log format directives. Assert
that the stored value is byte-identical to the validated input, that no
other record changed, that CLI output is escaped, and that log output is
unaffected.

## 13. Crash safety (T-CRASH)

A fault-injecting file layer stops a write at each step (create, partial
write, before sync, before rename, before directory sync) during: identity
creation, storing the Onion Service key, contact acceptance, endpoint
update, queue update, file completion. After restart: no partial contact,
no epoch lower than before, no new identity where one existed, the vault
opens.

## 14. CI

On every change:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo +1.85.1 check --workspace --all-targets --locked
    cargo +1.85.1 test --workspace --locked
    cargo +stable test --workspace --locked
    cargo deny check
    cargo audit

Formatting, lints and the first test run use the developer toolchain. The
MSRV and current stable runs are separate jobs.

Scheduled: fuzzing of every target for a fixed time, with the corpus
cached. Dependency updates are reviewed by a person; nothing is merged
automatically.

Mutation testing is run before a phase is called done: a list of single
faults in the checks that matter (a comparison removed, a bound moved, a
check skipped) is applied one at a time, and each has to make a test fail
unless its entry says why it cannot. A fault in authentication logic that
no test notices blocks the phase. The lists, the runner and the expected
results are in `mutation/`; see `mutation/README.md`.

## 15. Exit criteria

| Phase | Before it is called done |
| --- | --- |
| 1 | T-FRAME, T-FIELD, T-CARD, T-TEXT, T-FILE-NAME, T-PROTO-STATE, T-ORACLE-1 to 5 pass; property tests in place; fuzz targets for every decoder run clean for a fixed budget |
| 2 | handshake test vectors committed and reproduced; T-HS, T-HS-PIN, T-BIND, T-STALE, T-FRAME-AUTH and T-LIMIT pass; no mutation of an authentication check survives; fuzz targets for the handshake and for encrypted frames run clean for a fixed budget |
| 3 | T-SOCKS, T-CTRL pass; T-NET-1 passes; two-node chat over a private Tor network |
| 4 | T-CONTACT, T-ID, T-DUP, T-CONFIRM, T-ORACLE-6 to 8, T-CRASH, T-INJ pass |
| 5 | T-FILE passes; transfer fuzzing clean |
| 6, 7 | platform matrix passes on the current release |
| 9 | T-RES passes with measurements recorded |
