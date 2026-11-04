# Test plan

Status: the protocol core, the session layer, the Tor adapter and the
contact store with its vault have tests; everything else is a plan.

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
private Tor network test (`tests/tor-network/`): a message each way over a
Chutney network, a service published again from its key, a dial without
SOCKS, a lost control connection, every node under `strace`. Covered
now: T-SOCKS, T-CTRL, T-NET-1, and the private network test, which
passed in the final verification of Phase 3.

What Phase 4 added, which passed its final verification on `970316f`:
in `monolith-protocol`, the exhaustive table of the credential decision
against a model written from PROTOCOL.md section 11.4
(`credential::tests::table`) and the decision about a card for every kind
of record and every context (`contact::tests`); in `monolith-storage`,
the vault with its format checked against an independent reader and
writer and a pinned test vector (`tests/vault_format.rs`), a crash model
of the directory, and the payload records; in `monolith-core`, the
contact store (`tests/store.rs`: restart, a crash at every step of every
critical change, races), admission and retirement through the store
(`tests/credentials.rs`), invitations (`tests/invitations.rs`), several
identities (`tests/identities.rs`), the publication supervisor
(`tests/supervisor.rs`), what peers observe (`tests/observability.rs`),
a source scan (`tests/structure.rs`), the write job of the vault
(`persist::tests`: a cancelled wait, a failed write, many waiters on many
threads) and snapshots taken while operations run at the bounds
(`contacts::tests`); two more fuzz targets
(`vault_payload`, `contact_store`); the Phase 4 mutation list; and the
private Tor network test extended to nodes over a vault. Covered now:
T-INV-1 to 11, T-MI-1 to 11, T-CRASH for the vault and the contact store,
T-CONTACT-1 and 3, T-CONTACT-2 between two stores (`common::befriend`:
both import the other's card, one session makes both accepted), T-ID-2
and 3, T-CONFIRM-1, T-ORACLE-6, the later
sessions of T-ORACLE-7, T-INJ for the text the vault stores, and T-BLOCK.

What Phase 5 added, verified on `91bf22c`: in
`monolith-storage`, the message store file (`messages.rs`: a crash at
every step of a write and of a creation, a changed byte anywhere, the
largest payload) and its records (`message_records.rs`), and the
identifiers of payload version 2 of the vault; in `monolith-core`, the
message state machine (`messages::tests`), the session manager
(`sessions::tests`: both sides dialing at once, in every order of
arrival, keep the same session), the messages of an identity step by
step over links (`tests/messages.rs`, section 3b: every crash boundary
of delivery, duplicates, isolation, lifecycle) and the runtime over the
in-memory Tor (`tests/runtime.rs`); in `monolith-runtime`, the boundary
for a front end; two more fuzz targets (`message_store`,
`message_sequence`, the second against a model written apart from the
state machine); the Phase 5 mutation list (MR1 to MR57); and the private
Tor network test extended to the runtime. Covered now: T-MSG-1 to 13.

What Phase 6 added, verified on `68ac655`: the desktop client's tests
(section 3c): its UI model against the stale-instance scenarios of the
brief and in property tests against a separate model of the runtime, the
components of its design system driven by input, the contrast of its
palette, the screens (a source scan, the smallest window, an idle window,
the keyboard), its bridge against the runtime on the in-memory Tor, its
crate boundary, its files; two more fuzz targets (`ui_present`,
`ui_model_sequence`); the Phase 6 mutation list (UI1 to UI160); the
workflows of the brief run by script against the runtime; frame
measurements; and the private Tor network test extended to the client: two
clients driven through their window code become contacts through an
invitation and exchange a message each way over Tor. The networking itself
stays proved by the runtime steps of Phase 5; this step shows that the
client's commands and events drive that runtime, with the configuration
the client makes of its Tor settings. Covered now: T-UI-1 to 16.

What Phase 7 added, verified on `bbf1279` (the full verification on
`740b671`, whose production code it has): a regression test for every
fix, each failing before it; property tests against models written
apart from the code, for the rates, the file messages of a session, a
store above a lowered limit and the notices of another identity
(`DESIGN_QUESTIONS.md` P7-22); runtime tests of the dial plan during a
rotation, of file messages, of the probes and Pings of a session and of
the rates; tests that run on Windows only, for the checks of local
files, the renames another process blocks and the name of the cookie
file; the fuzz targets `ui_present` and `message_store` held to the
larger set of marks and to the ceilings of the store; the Phase 7
mutation lists, HR1 to HR85, 14 of them in a list for Windows that the
runner now runs there (`phase7w`); `mutation/affected.py`, which selects
the faults a change can affect; and `cargo doc` with warnings denied in
the CI.

What Phase 8 added: the tests of file transfer (section 3d). In
`monolith-core`, the rules of a session (`exchange::tests`), the state
machine and a property test of its transitions (`transfers::tests`), the
transfers of two runtimes over the in-memory Tor (`tests/transfers.rs`),
a property test of two runtimes against a model of their own
(`tests/transfer_model.rs`), the crash sweeps, the hostile sender, the
races, fairness and boundedness (`tests/transfer_restarts.rs`); in
`monolith-storage`, the formats of ADR 0014 against a reader and a writer
written from the ADR (`tests/transfer_format.rs`) and the save of a
received file (`tests/export.rs`); in `monolith-ui-model`, the cards of
transfers and transfers in the property test; two more fuzz targets
(`transfer_sequence`, `transfer_store`); the Phase 8 mutation list (FT1 to
FT62); and the private Tor network test extended to files (steps 22 to
32). Covered now: T-FT-RULES to T-FT-NET, and T-CRASH for file completion.

The rest is not written yet.

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
   is a violation. Without transfers (P7-4), only a FileAbort or the same
   offer again crosses a rejected offer; a chunk, a completion or an
   answer of the offerer for it ends the session (P7-23). A property
   test runs any sequence of file messages of one session against the
   history of the offers it rejected (`transfers::tests`, P7-22).

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
of the contact store, and these tests run against it on the in-memory Tor
(`tests/invitations.rs` in the core, one test per item or pair of items);
the card part of T-INV-1 is also in `card::tests` (cards that differ only
in their capability are each valid, have their own signature and do not
conflict), and T-INV-8 and T-INV-11 also in the private network test.

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

Local identities (T-MI)

`ARCHITECTURE.md` section 1.1, invariants S39 to S46. Since Phase 4 an
installation holds several identities, and these tests run against it:
`tests/identities.rs` in the core (T-MI-1 to 4, 7 to 11),
`tests/link.rs` (T-MI-5 and 6: each of two local identities answers at
its own service and dials the same contact with its own isolation
group), `tests/system.rs` in the Tor crate (two publications coexist on
one backend and a stream reaches only its own), and the private network
test (two identities of one node).

1. An identity whose identity, transport or Onion Service key equals one
   of another local identity is refused at creation, import and restore.
2. Contact state is per identity: a peer accepted by A is a stranger to
   B, and A's verification marks, aliases, pending requests and former
   contacts do not exist for B.
3. Blocking a peer for A leaves B's handling of that peer unchanged.
4. A capability issued by A admits no request to B, and revoking it at A
   changes nothing at B.
5. The core keeps one isolation group per local and remote identity: A to
   Bob, A to Claire and B to Bob use three groups.
6. With A and B published, every stream to A's service runs A's party,
   policy and lookup, and every stream to B's runs B's.
7. What a peer of A receives (messages, their number and order, the close
   condition) does not change with the number or state of other local
   identities, and a flood of strangers at B leaves A's budgets and
   answers unchanged.
8. Every dial names its local identity; dials of A and B use their own
   parties, records and groups.
9. Deleting B leaves every record of A byte for byte unchanged, and a
   backup of one identity restores its keys, capabilities and contacts
   together.
10. What arrives on a session of A is applied to A's state only: handed
   to B, or to an identity made again from A's keys after A was deleted,
   it is refused and changes nothing.
11. A handle of an invitation of A, used at B or at an identity made again
   from A's keys, finds nothing: no card, no revocation.

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
   not belong together: a party is only issued from an identity key, and
   adopting a card does not compile (S47). A party that holds another
   identity's transport key acts only as its own identity, as responder
   and as initiator.
4. A peer that authenticates with one key and presents a card of its own
   identity that states another, its successor card with the current key
   or its current card with the successor key, is refused (F3).

Stale cards (T-STALE): the standing of a peer for every record and every
relation between the card of the session and the credentials held; a
retired transport key, also under a higher epoch, against a contact that
promoted the new one; a dialed card whose key was retired while the dial
was in progress; a newer card with the active key keeps the contact.

Credentials (T-CRED), in `credential::tests`, `tests::credentials` and
`tests::contacts` of the session crate, and `tests/credentials.rs` of the
core:

1. A session of the active key announces a successor: it is authorized,
   and the active key keeps working.
2. The successor is proven in a handshake: it is promoted and the old key
   retired.
3. A session of the old key after the promotion is withdrawn and delivers
   nothing, also a frame already in its buffer or one still on the
   stream; a waiting link wakes up and sends Close.
4. A new session of the old key after the promotion is not a contact
   session.
5. A newer key without announcement is pending, gives no standing and
   does not lock out the active key.
6. Confirming exactly the pending card promotes it; any other card is
   refused.
7. A card of another identity never becomes a successor; a card with a
   bad signature or a key other than the proven one never reaches the
   credentials (T-BIND).
8. An EndpointUpdate on a session of another key authorizes nothing.
9. An import for an accepted contact never replaces the key; for a
   requested contact it replaces the card and its capability (P10).
10. Two identities rotating at the same time complete while the old keys
    answer, also when their first dials with the new keys cross; when both
    old keys are gone, nothing is taken over and an out-of-band card that
    the user confirms recovers.
11. The duplicate rule never keeps a session of a retired key over one of
    the active key.
12. A key retired while a dial is in progress gives the dialed session no
    contact standing, and message 3 with the local identity is not sent:
    admission reads the contact state after message 2.
13. Property: over random sequences of admissions, announcements,
    confirmations and imports the invariants hold, the active epoch never
    decreases, and the active key changes only by promoting a proven
    authorized successor, by confirming the pending one, or by a
    replacement for a requested contact.
14. A card a peer presents does not displace a pending card the user
    imported; a card at the epoch of the authorized successor that states
    something else is a conflict.
15. A withdrawn link fails at once on every later call, also when it was
    withdrawn before it was polled; a withdrawal seen first by a send
    writes the Close; a withdrawal while a send is pending ends it without
    a Close; a stale or pending session is not withdrawn early; a dropped
    link, one `answer` refused after the admission, and an admission
    function that fails report that they ended.
16. Message 3 is not written when the session is withdrawn right after
    the admission, nor after a withdrawal while its write is stalled; the
    bytes accepted before stay, nothing follows them.
17. An older card of the active key gets message 3 and a contact session;
    a conflicting card of the same epoch, a pending or retired key, and a
    record deleted, declined or blocked during the dial get no message 3
    and no session, and take no slot for strangers, also when none is
    free. A refused dial returns the admission, so a conflict is seen
    locally while the peer sees only the stream close; the active card is
    not rolled back by an older card of its key.
18. A promotion made at message 2 stands when message 3 cannot be
    written.
19. A proven key older than the successor announced through the active
    key is stale: no message 3, and nothing is recorded. A pending key
    gets no message 3; its card is held for the user, as inbound.
20. Message 3 and the slot for strangers follow the standing of the
    session, whatever admission the admission function returns.

Session lifetime (T-LIFE), in `tests::deadlines` of the session crate and
the unit tests of `link`:

1. A frame sent a byte at a time, each byte within what a timeout per
   read would allow, ends at `FRAME_READ_TIMEOUT` from its first byte.
2. No complete frame for `IDLE_TIMEOUT` ends the session; a complete
   frame moves the limit, bytes of an incomplete one do not.
3. A peer that is not a contact and sends nothing is closed at
   `UNKNOWN_FIRST_MESSAGE_TIMEOUT` and gives back its slot.
4. A session that stays unconfirmed is closed at
   `UNKNOWN_SESSION_TIMEOUT`.
5. The age limit ends a session that only receives part of a frame.
6. After a timeout, a read error or a write error, every later receive
   and send fails at once, and no late frame is delivered.
7. The first message of a peer that is not a contact is returned with
   the Close already written and the slot back, whether or not the caller
   closes; in the session alone that Close is due at once. A Close from
   the peer ends the link at once and gives back its slot.
8. A link that ended, by a withdrawal or a read error, gives a stream
   whose shutdown never completes `FRAME_WRITE_TIMEOUT` once; every later
   receive, send and close returns at once.

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

## 3b. Messaging (T-MSG)

The delivery semantics of `adr/0007-delivery-semantics.md`, the store of
`adr/0008-message-store.md`, the sessions of
`adr/0009-session-manager.md`. Each is a deterministic test: the steps of
two nodes are taken one at a time over links, a crash is placed with the
model of `MemoryDir` or a held write, and the runtime runs with paused
time.

| Id | What | Where |
| --- | --- | --- |
| T-MSG-1 | A message queued while the peer or Tor is away is delivered once it is back, through Tor | `tests/runtime.rs` |
| T-MSG-2 | A message is stored once and delivered only by its acknowledgement; a second acknowledgement and an unknown one change nothing | `tests/messages.rs` |
| T-MSG-3 | Crash boundaries: the sender before sending, after writing, before the delivery is durable; the receiver before the message is durable, after it and before acknowledging; a delivered message stays delivered; a crash at every step of queueing; nothing is offered for sending before it is durable | `tests/messages.rs`, `tests/structure.rs` |
| T-MSG-4 | Duplicates: the same identifier twice, concurrent copies on two sessions, a retry after a restart, a retry over a replacement session, a retry of a deleted message | `tests/messages.rs`, `tests/runtime.rs`, `messages::tests` |
| T-MSG-5 | Isolation: an acknowledgement reaches only the conversation of its session; the identities of one installation keep messages, queues and sessions apart | `tests/messages.rs`, `tests/runtime.rs` |
| T-MSG-6 | Lifecycle: block, delete, a contact made again, a new card of the same contact, an identity deleted and restored, a deleted identity that stops dialing, a block or a deletion during a dial; a record that ends during a session, or a store that fails during a dial, announces no retry; a block or a deletion for a record made again before its work runs ends nothing; a send for a record deleted while it waits for the vault queues nothing, nor one for a record blocked on another thread before its message is queued; a session the scheduler waited on announces its retry; a connect dials no further card once the runtime shuts down (P7-23) | `tests/messages.rs`, `tests/runtime.rs` |
| T-MSG-7 | Sessions: both sides sending at once end with one session and every message once; the duplicate rule in every order; a probe answered or not, answered while the session is not read, asked while a Ping is outstanding, of a session whose key was retired meanwhile; a Pong after the probe was decided; a Ping not answered in time; a contact that keeps no idle session open with messages held already or acknowledgements of nothing | `tests/runtime.rs`, `sessions::tests` |
| T-MSG-8 | Reconnecting: nothing is dialed without a queued message; a failed attempt is not repeated at once; an acknowledgement that does not come closes the session `MESSAGE_ACK_TIMEOUT` after the one send of the message, whatever is queued behind it, and the message is sent again later, with growing delays; a session that ends with a message in flight reports nothing delivered; a session that ends with its service leaves no registration behind; an idle session ends; a failed message store or vault ends every session and attempt and is reported at once; a runtime dropped without a shutdown stops; a send its caller stops waiting for completes, and a shutdown waits for it; a send once the runtime shuts down, or one a shutdown cuts short, says so and is not a failed store; an identity without a key is reported so, and stays so; an inbound session that timed out does not reset the delays; availability is reported in turn | `tests/runtime.rs`, `runtime::tests`, `supervisor::tests` (the jitter) |
| T-MSG-9 | Limits: text at `MAX_CHAT_TEXT_LEN` - 1, at it and + 1, empty and with forbidden characters, checked before anything is stored; the queue of a contact and of an identity; a conversation in bytes | `tests/runtime.rs`, `tests/messages.rs`, `messages::tests` |
| T-MSG-10 | Storage: a store that does not authenticate fails the installation and is left as it was; a store above any lowered limit of a queue or of the messages is read and written back whole, and takes a message in exactly when it holds fewer (a property test, P7-22); a vault of Phase 4 is opened with identifiers and an empty store; a vault with identities and no store opens and reports it, a vault of Phase 4 beside a store with messages is refused; a crash at every step of making the store leaves an installation that opens; corrupt, truncated, out of order or impossible contents are refused | `tests/messages.rs`, `message_records.rs`, `messages.rs` in `monolith-storage` |
| T-MSG-11 | Model: any sequence of queueing, receiving, acknowledging, deleting, removing an identity and restarting leaves the state a separate model holds; any sequence of sending from either side, crashes and restarts of either side, Tor going and coming back, closed sessions, time passing and a block leaves the runtime delivering, in order and once, what a model of the sent texts says | `tests/message_model.rs`, `tests/runtime_model.rs` (property tests), fuzz target `message_sequence` |
| T-MSG-12 | Diagnostics: no message text in `Debug` output | `messages::tests`, `api::tests`, `message_records.rs` |
| T-MSG-13 | Structure: a message is delivered only by an acknowledgement a link received, acknowledged only once `take_in` returned, sent only once durable; the boundary of a front end hands out no authority | `tests/structure.rs` |

## 3c. Desktop client (T-UI)

The client of `DESKTOP.md`. The UI model and the components are tested
without a window: egui runs headless, and what it draws can be rendered
to images by `monolith-ui-snapshot`.

| Id | What | Where |
| --- | --- | --- |
| T-UI-1 | Stale instances: a late event of one identity, a contact of another identity accepting (no notice, P7-17), a contact made again, a confirmation after the selection moved, a replaced request, an invitation of another identity, a Delivered of another conversation, an answer to no command in flight, an unblock of someone not listed or answered while another identity is shown | `monolith-ui-model/tests/model.rs` |
| T-UI-2 | Reconstruction: missed events read everything again; a burst of events makes one read; a deleted identity takes its state with it | `monolith-ui-model/tests/model.rs` |
| T-UI-3 | Model: random sequences of actions, events and late answers against a separate model of the runtime keep the invariants of S59 and S60; an event raises a notice only when it is a contact of the identity shown accepting (P7-22) | `monolith-ui-model/tests/properties.rs`, fuzz target `ui_model_sequence` |
| T-UI-4 | Presentation: rendering limits, visible marks for invisible characters (tag and other format characters included, P7-14), previews, names | `present.rs` tests, fuzz target `ui_present` |
| T-UI-5 | Components: buttons, text fields (the secret field keeps no history; a byte limit cuts at a character), the composer and the text fields (an Enter while an input method composes sends nothing and submits nothing, P7-19, P7-23), the avatar of a request and the label of a row (a screen reader hears a sender's name only in the box of their words, a preview after the markers, P7-23), bubbles and their status (a run without a break opportunity broken inside the bubble), modals, rows, the identity switcher, driven by pointer and keyboard | `monolith-ui-kit/tests/` |
| T-UI-6 | Contrast of every text on what it is drawn on, the initials of avatars included, both themes | `monolith-ui-kit/tests/contrast.rs` |
| T-UI-7 | Screens: "Check Tor" names the control port where Tor may be running (P7-15); no stock widget, no style of their own, no run of spaces in a text; the smallest window scrolls to everything; a settled window asks for no frame; the keyboard shortcuts; a request's name is in the box of the sender's words, never a title or part of a sentence of the client, whatever it holds (P7-2); a message's mark is the runtime's status; the people an identity blocked are listed and unblocked after a question, a request is declined after one; a failed store is said once; a name field takes no more than a name may hold (P7-14); a data folder refused at the start is said so (P7-17); a waiting contact is said to wait for the same thing everywhere; a passphrase buffer never moves | `monolith-ui/tests/`, `requests::tests`, `conversation::tests` |
| T-UI-8 | Bridge: every command against the runtime on the in-memory Tor, a number of one identity naming nothing of another, a number of someone unblocked naming nothing, also once they are blocked again, paging of long conversations, the runtime stopping with the window, a runtime thread that panics said to have stopped and waking the window, the window dropped without waiting for the runtime or the file thread, which are waited for after it | `monolith-desktop/tests/bridge.rs`, `window::tests` |
| T-UI-9 | Boundary: the UI crates reach no Monolith crate, the desktop reaches the core only through the runtime | `monolith-desktop/tests/boundary.rs` |
| T-UI-10 | Files and places: the settings file (a value cannot add a line, what is written is read back, the size of the window kept whatever the zoom), the folder of a saved card chosen on the thread of the files (P7-16), the root of a drive resolved before anything on it is looked at and a downloads junction not followed (P7-23), Tor settings without a cookie path refused, a card never replaces a file, a card file is a small regular file, never read through a link, nothing else opened (a pipe, also behind a link) and a name given to something else between the look and the open refused, Tails places, fallback fonts that do not read are left out | `monolith-desktop` unit tests |
| T-UI-11 | No URL leaves a frame | `window::tests` |
| T-UI-12 | Workflows A to G of the brief, with pictures: two clients with profiles on disk, the real bridge and runtime on the in-memory Tor, in real time | `monolith-desktop/examples/scenarios.rs` (run by hand) |
| T-UI-13 | A card or an invitation (P6-20): adding one says which it was, and the contact keeps saying it, also after a restart; a plain card's request is dropped in invitation mode until the card of its sender is added too; the model keeps "Add contact" open after a plain card, unless the contact had accepted; the dialog, the conversation and the own card say what each case waits for | `monolith-core/tests/front_end.rs`, `monolith-desktop/tests/bridge.rs`, `monolith-ui-model/tests/model.rs`, `monolith-ui/tests/views.rs` |
| T-UI-14 | Closing: a session that saves nothing, or messages not delivered (not those of an ended conversation), make a close by the system ask first; the close is cancelled and the window stays shown, also when asked again, until Close Monolith; Cancel, Escape and Enter keep it; the words count the messages; with nothing to lose the window is hidden at once | `window::tests`, `app::tests`, `dialogs::tests`, `monolith-ui/tests/views.rs`, `monolith-ui-model/tests/model.rs` |
| T-UI-15 | Delivery: a waiting delivery says once that both sides must be open, and when the next attempt is due, counted from when the attempt failed, said again when the words change and not at all while Tor is down; typing while a message is being queued stays in the draft once it is queued, and a refused send keeps all of it | `monolith-ui-model/tests/model.rs`, `conversation::tests`, `monolith-ui/tests/views.rs`, `idle.rs` |
| T-UI-16 | Over Tor: two clients, one process each, driven through their window code (screens, UI model, bridge, runtime, file thread) over Tor A and Tor B of the private network, with the Tor configuration the desktop makes of its settings, so that a dial waits until Tor lists the SOCKS endpoint (`GETINFO net/listeners/socks` against a real Tor). A Tor check that says Tor works, an identity each, an invitation saved to a file and dropped on "Add contact", a request accepted, a message each way shown Delivered to its sender and Received by the other, a clean close; every connection to loopback or a Unix socket. Without Tor endpoints the same script runs both clients on the in-memory Tor | `monolith-desktop/examples/tor_smoke.rs`, step 21 of `tests/tor-network/two-node.sh` |

## 3d. File transfer (T-FT)

The transfers of `adr/0013-file-transfer.md` and the staging of
`adr/0014-transfer-staging.md`. Paths are under `crates/monolith-core`
unless a crate is named. The runtime tests run over the in-memory Tor with
paused time; a crash is placed with the model of `MemoryDir`, a held write
or a gated source. Where one side is driven by hand, the test writes what
that side sends and reads what the other answers.

| Id | What | Where |
| --- | --- | --- |
| T-FT-RULES | The rules of a session: a file message for a transfer never offered is a violation, so is a second offer; an offer refused is remembered and only an abort or the same offer crosses it; only the last `MAX_ENDED_TRANSFER_IDS` ended transfers are remembered; chunks follow the acceptance with no gap and no overlap, each of `min(L, bytes left)`; a completion counts only with every byte received; an acceptance offset that is stale, in the future, misaligned or of another transfer is refused; the offerer writes at most the window beyond the acknowledged offset, one transfer at a time, the one accepted first; chunks on the way when this side ended a transfer are discarded within the window; a completion that crossed a cancellation still counts once; progress is acknowledged after two chunks or after its interval | `exchange::tests` (`chunks_follow_the_acceptance_with_no_gap_and_no_overlap_to_the_size`, `an_acceptance_offset_that_is_stale_future_misaligned_or_of_another_transfer_is_refused`, `the_offerer_writes_at_most_the_window_beyond_the_acknowledged_offset`, `a_completion_that_crossed_a_cancellation_of_this_side_still_counts_once` and the rest of the file) |
| T-FT-STATE | The state machine: each transition with its authority, a completion with another digest fails the transfer, a cancellation is told only if the offer may have been seen, a receiver that completed before reading a cancellation delivered the file, the limits of offers, of active transfers and of staged bytes, room made from ended records in the order of the ADR and never from others, the stall deadline written at most once an hour, a held transfer answered with nothing, a blocked contact's transfers cancelled and a deleted one's removed, a contact made again inherits nothing, reconciling with the vault, a flag that the peer read an answer cleared only for that answer. A property test runs any sequence of transitions on a few transfers of two contacts of two identities against the table: records valid and read back as written, limits never passed, a state reached only along an edge, a record not ended never removed but by the end of its contact or identity, a blob free exactly when no record names it | `transfers::tests` (`a_contact_made_again_inherits_nothing`, `a_completion_with_another_digest_fails_the_transfer`, `a_receiver_that_completed_before_reading_a_cancellation_delivered_the_file`, `room_is_made_from_ended_records_in_the_order_of_the_adr_and_never_from_others` and the rest), `transfers::tests::model` (`every_sequence_of_transitions_keeps_the_rules`) |
| T-FT-FORMAT | The formats of ADR 0014 against a reader and a writer written from the ADR alone: a blob and a transfer store built from fixed inputs open with the code and are pinned by their SHA-256; what the code writes opens by the ADR; every segment has a nonce of its own and no plaintext shows. The blob: a staged file reads back at every size boundary, a source with fewer or more bytes than its size stages nothing, a changed, moved or foreign segment does not authenticate, padding that is not zero does not authenticate, the scan after a crash stops at the first segment that does not authenticate, the reader fails at the end when the digest differs. The store: records of every state read back, every rule of a record is checked, a crash during a write leaves the previous or the new records, a store under another key or changed does not open | `tests/transfer_format.rs` in `monolith-storage` (`a_blob_built_by_the_adr_opens_and_is_pinned`, `a_blob_the_code_writes_opens_by_the_adr`, `every_segment_the_code_writes_has_a_nonce_of_its_own_and_no_plaintext_shows`, `a_transfer_store_built_by_the_adr_opens_and_is_pinned`, `a_transfer_store_the_code_writes_opens_by_the_adr`), `blob::tests` and `transfers::tests` in `monolith-storage` |
| T-FT-SAVE | Saving a received file: whole under its name or not at all, an existing name never replaced, also one taken while the file is written, a source that fails leaves nothing, a place that is not a folder of this computer is refused before anything is written (on Windows a share, a device, a stream or a junction), a name a peer suggests saves in the folder chosen and nowhere else, a file with a Unicode name deep in folders, an empty file | `tests/export.rs` in `monolith-storage` (`a_file_is_saved_whole_under_its_name_and_nothing_else_is_left`, `an_existing_name_is_never_replaced`, `a_name_taken_while_the_file_is_written_is_not_replaced`, `a_source_that_fails_midway_or_at_its_end_leaves_nothing`, `a_name_a_peer_suggests_saves_in_the_folder_chosen_and_nowhere_else`), `tests/transfers.rs` (`a_file_offered_by_its_path_is_sent_as_staged_and_saved_where_the_user_chooses`, `a_file_of_a_unicode_name_deep_in_folders_is_offered_and_saved_under_that_name`, `a_path_that_is_not_a_regular_file_of_this_computer_is_never_offered`, `an_empty_file_is_sent_and_completed`) |
| T-FT-RUNTIME | Two runtimes over the in-memory Tor, one side sometimes driven by hand: a file offered, accepted, sent, verified and saved whole; a rejected offer; the window; resumption from what the receiver holds and nothing else; a digest that does not match; a completion before every byte arrived; a peer of Phase 7 that rejects the offer while chat goes on; cancellations by either side, told once, what crossed them discarded, one the peer could not be told told with the next offer; a block or a deletion; a Pong that confirms what its Ping followed and nothing written after it; a transfer that goes on over the session that replaces its own and over a session of the new key of its contact; offers above the pending limit rejected without a record; an offer not decided in time expires; a sender of Phase 8 sends a peer of Phase 7 nothing that ends its session | `tests/transfers.rs` (`a_file_is_offered_accepted_sent_verified_and_saved_whole`, `the_sender_never_has_more_than_the_window_beyond_what_the_receiver_acknowledged`, `a_transfer_resumes_from_what_the_receiver_holds_and_nothing_else`, `a_file_that_does_not_match_its_digest_fails_and_is_never_offered_as_received`, `a_peer_of_phase_7_rejects_the_offer_and_chat_goes_on`, `a_sender_of_phase_8_sends_a_peer_of_phase_7_nothing_that_ends_its_session`, `a_pong_confirms_what_its_ping_followed_and_nothing_written_after_it`, `a_deleted_contact_loses_its_transfers_and_a_contact_made_again_inherits_none` and the rest) |
| T-FT-MODEL | Two runtimes against a model of their own: persistent installations over the in-memory Tor, and any sequence of offering files of several sizes from either side, accepting, rejecting, cancelling from either side, removing, saving, crashing either side at once or at a later step of its storage under every crash outcome, Tor going away and coming back, closing the sessions, letting time pass, and one side blocking or deleting the other. The model is what each user did and the transitions a transfer may make, written from the table of the ADR; nothing of the state machine says what is right. After every step and as a side stops at a crash: every transfer listed is one the model knows, with one record each of that side's identity and contact record; a status moved only along the table and an ended transfer never goes on, what was shown stays shown across every crash (the one exception is a cancelled outgoing transfer that the receiver had completed); each status has its authority; the bytes shown never pass the size; one shown completed at its sender is shown received at its receiver unless that user removed it; a command is refused only for a reason the truth gives; a received file reads back exactly what was offered, once. At the end, with Tor back and the offers accepted, a day passes: every transfer is ended at both sides, a file is received at one side exactly when it is completed at the other and reads back whole | `tests/transfer_model.rs` (`transfers_between_two_runtimes_follow_the_model`, 32 cases) |
| T-FT-CRASH-RECV | The receiver crashes at every step of its storage: a receiver that crashes resumes from what it holds durably, never less; a crash at every step of receiving shows nothing it takes back and the file completes whole, under every crash outcome; a receiver that crashes after its completion acknowledges it again with one file; one that crashes before verifying verifies again; what is shown of a transfer is durable (an offer whose record is not written is not listed) | `tests/transfer_restarts.rs` (`a_crash_at_any_step_of_receiving_shows_nothing_it_takes_back_and_the_file_completes_whole`, `a_receiver_that_crashes_resumes_from_what_it_holds_durably`, `a_receiver_that_crashes_after_its_completion_acknowledges_it_again_with_one_file`, `a_receiver_that_crashes_before_verifying_verifies_again`, `what_is_shown_of_a_transfer_is_durable_and_no_crash_takes_it_back`) |
| T-FT-CRASH-SEND | The sender, and the decisions, at every step: an offer the receiver read is held by the sender after a crash at any step of its storage; a decision (acceptance, rejection) the sender read is held by the receiver after a crash at any step; a cancellation the receiver read is held by the sender after a crash at any step; nothing told is lost to a crash. A sender that crashes after its offer offers it again and resumes; one that crashes before recording the completion is completed again; a staging cut short by a crash leaves no offer and no blob; a source that gives fewer or more bytes than its size offers nothing; a save cut short by a crash is reported and the file kept; a staged copy that is damaged or missing fails when it is saved | `tests/transfer_restarts.rs` (`an_offer_the_receiver_read_is_held_by_the_sender_after_a_crash_at_any_step`, `a_decision_the_sender_read_is_held_by_the_receiver_after_a_crash_at_any_step`, `a_cancellation_the_receiver_read_is_held_by_the_sender_after_a_crash_at_any_step`, `a_sender_that_crashes_after_its_offer_offers_it_again_and_resumes`, `a_sender_that_crashes_before_recording_the_completion_is_completed_again`, `a_staging_cut_short_by_a_crash_leaves_no_offer_and_no_blob`, `a_source_that_gives_fewer_or_more_bytes_than_its_size_offers_nothing`, `a_save_cut_short_by_a_crash_is_reported_and_the_file_kept`, `a_received_file_whose_staged_copy_is_damaged_or_missing_fails_when_it_is_saved`) |
| T-FT-HOSTILE | A hostile sender (a property test, 48 cases): offers the file again with its size or one more, resumes from the offset answered or from where it was, sends chunks out of order, of the wrong length or with a byte changed, moves on or back, completes with the digest of what it sent or of another file, aborts, closes the session, while the receiver's user accepts and the receiver crashes and starts again. A file is shown received only with exactly the size offered and a digest the sender declared, from bytes staged durably, and so it stays; the bytes shown never pass the size; the status moves only along the table. That is integrity: the file is the one the sender declared, whole. Nothing in the test says it is safe. A sender that ignores the window gets no more staged than the size it offered and holds up no chat | `tests/transfer_restarts.rs` (`whatever_a_sender_sends_a_file_is_shown_received_only_whole_and_verified`, `a_sender_that_ignores_the_window_stages_no_more_than_the_file_and_holds_up_no_chat`), `tests/transfers.rs` (`an_acknowledgement_that_is_misaligned_beyond_what_was_sent_or_of_another_transfer_ends_the_session`, `a_sender_that_resumes_from_another_offset_than_the_receiver_never_completes`) |
| T-FT-RACE | Races forced with held writes and gated sources: the user cancels before the receiver's completion arrives, the completion arrives while the write of the cancellation is held, the cancellation comes while the write of the completion is held, or the user cancels after the completion; each ends as one of the two serial runs does, completed, so a cancellation never takes back a file the receiver holds. An acceptance as the contact is blocked ends in one order or the other. An identity deleted while its file is staged (a source that waits until it is let go) gets no offer and keeps no blob; a save under way as its identity is deleted is cancelled and the blob then goes; a contact deleted while its file is saved cancels the save and leaves no file or part of one; what is shown is durable while a write is held | `tests/transfer_restarts.rs` (`a_cancellation_of_the_sender_that_meets_the_completion_ends_completed_in_every_order`, `an_acceptance_as_the_contact_is_blocked_ends_in_one_order_or_the_other`, `an_identity_deleted_while_its_file_is_staged_gets_no_offer_and_keeps_no_blob`, `a_save_under_way_as_its_identity_is_deleted_is_cancelled_and_the_blob_then_goes`, `what_is_shown_of_a_transfer_is_durable_and_no_crash_takes_it_back`), `tests/transfers.rs` (`a_contact_deleted_while_its_file_is_saved_cancels_the_save_and_leaves_no_file_or_part_of_one`) |
| T-FT-FAIR | Fairness: a contact flooding its chunks holds up neither the chat nor the file of another contact; a chat message of the sender is not held behind its chunks, is written after at most one chunk, and is acknowledged before the next chunk is read; chat goes both ways while a large file is sent; two files to one contact go one at a time in the order accepted and never mixed | `tests/transfer_restarts.rs` (`a_contact_flooding_its_chunks_holds_up_neither_the_chat_nor_the_file_of_another`), `tests/transfers.rs` (`a_chat_message_of_the_sender_is_not_held_behind_its_chunks`, `a_chat_message_waiting_while_chunks_go_out_is_written_after_at_most_one_chunk`, `a_chat_message_between_chunks_is_acknowledged_before_the_next_chunk_is_read`, `chat_goes_both_ways_while_a_large_file_is_sent`, `two_files_to_one_contact_go_one_at_a_time_in_the_order_accepted_and_never_mixed`) |
| T-FT-BOUND | Boundedness (a property test, 16 cases): through sessions that end, offers repeated, a full disk, time passing and crashes, what a receiver keeps stays bounded by what it accepted, one record, at most one staged file, never more bytes than the size; while nothing can be written or no sender can be reached it neither writes nor dials more than a few times an hour; at the end the file arrives whole, and once removed leaves nothing behind. Also: a full disk holds the transfer until it is resumed or the hold ends; a dial for a transfer alone that brings no progress is repeated with growing delays; a transfer store that cannot be written stops everything until a restart; a session that carries a transfer lingers from its last progress | `tests/transfer_restarts.rs` (`a_receiver_stays_bounded_through_lost_sessions_repeated_offers_and_a_full_disk`, `a_full_disk_holds_the_transfer_until_it_is_resumed_or_the_hold_ends`, `a_transfer_store_that_cannot_be_written_stops_everything_until_a_restart`), `tests/transfers.rs` (`a_dial_for_a_transfer_alone_that_brings_no_progress_is_repeated_with_growing_delays`, `a_session_that_carries_a_transfer_lingers_from_its_last_progress`) |
| T-FT-UI | The UI model with transfers: each card says where its transfer stands from the runtime's state alone; a file being prepared is never shown as offered or sent; bytes not known after a restart are never shown as 0; bytes an event gave are not shown over a later read that says more or fewer, nor after the runtime says it no longer knows them; a file goes only to an accepted contact; an action is taken only where its card offers it; a decision names its transfer; events and answers of one identity change nothing shown of another; a name is drawn with its control characters shown; an interrupted save is told once. The property test (1024 cases) adds transfers to the environment, which offers, accepts, sends, holds and ends files, and starts again without knowing how far a transfer got: a transfer action issues its command only for a transfer listed, in a state it applies to, that no command is busy with; once no read lists a transfer it is never listed again; every transfer listed has one card, in the conversation of its record, that says what the read said; the name is drawn on one line with no control character and no mark that hides text or turns its direction; bytes shown were said by the runtime for that handle and are at most the size; once shown ended, a transfer is never shown under way again nor in another ended state, but for a cancelled one the peer then completed | `monolith-ui-model/tests/transfers.rs` (`each_card_says_where_its_transfer_stands_from_the_runtime_alone`, `a_file_being_prepared_is_never_shown_as_offered_or_sent`, `progress_is_kept_from_events_until_a_later_read_and_never_shown_as_zero`, `bytes_the_runtime_no_longer_knows_or_has_passed_are_not_shown_from_an_event`, `a_read_asked_after_an_event_is_believed_also_when_it_says_fewer_bytes`, `an_action_is_taken_only_where_its_card_offers_it`, `a_file_name_is_drawn_with_its_control_characters_shown`), `monolith-ui-model/tests/properties.rs` (`the_model_shows_what_the_runtime_said_and_acts_on_what_was_named`) |
| T-FT-FUZZ | Fuzz target `transfer_sequence`: four-byte operations on the transfers of two local identities with two contact records each, both directions (offers, decisions, the peer's answers and completions with the right digest or another, staged bytes, failures, hours or days passing, contacts blocked or deleted, identities removed, restarts through the encoded store) against a model written in the target from the table of the ADR: statuses move only along it, each with its authority, bytes within the size, limits of a contact held, a removed transfer not listed again while its record lasts, a restart lists the same transfers with the same statuses. Fuzz target `transfer_store`: the payload of a transfer store is refused or decodes to records that keep every rule of ADR 0014, and its encoding with the zero padding is the input | `fuzz/fuzz_targets/transfer_sequence.rs`, `fuzz/fuzz_targets/transfer_store.rs` (seeds: `fuzz_seeds` in `monolith-core` and `monolith-storage`) |
| T-FT-MUT | The Phase 8 mutation list, FT1 to FT62: the rules of a session (FT1 to FT9, FT62), the state machine (FT10 to FT21, FT45, FT46, FT55 to FT58), the sessions and the staging (FT22 to FT30, FT34, FT51 to FT54, FT59, FT60), staging, names and saving (FT31 to FT33, FT35, FT36, FT61), the desktop client (FT37 to FT44, FT47 to FT50), with the map of the candidates of the brief to faults in `mutation/README.md`. FT51 to FT53 and FT45 to FT50 are the code as it was before the fixes the model and property tests prompted. | `mutation/faults.py` (`PHASE8`), `mutation/README.md` |
| T-FT-NET | The private Tor network, steps 22 to 32 of `tests/tor-network/README.md`, with `dev-runtime` and generated files: 22 an offer by path with the file changed after (the bytes saved are those staged), chat during the transfer, a save refused over an existing file and no temporary file left; 23 a file of 64 MB with B killed and started again and then A, the first acknowledgement after the restart above the last before it; 24 every file node run checked for loopback and Unix sockets only, no DNS, no UDP; 25 a file each way at once of 16 MB and 12 MB with chat, with the chat latency, throughput, memory and CPU printed; 26 Tor A stopped for 20 seconds while 32 MB is on its way; 27 every session through a transport fault proxy that cuts each connection after a number of bytes between 256 KiB and 3 MiB, wherever that falls in a frame, and holds data back by up to three seconds (seeds 1 and 2); 28 B killed as it reports a file complete, a lost acknowledgement; 29 B deletes A while a file comes and adds A again, the new record being offered the file from its start; 30 B blocks A while a file comes, no chunk taken for two minutes after; 31 a peer built from the tag `phase-7-complete` (`MONOLITH_PHASE7_BIN`) rejects A's offer once with no violation, and chat goes both ways after it; 32 a key rotation of B while a file comes, the file going on over a session of the new key to the same contact record, and a contact made again holding none of the old transfers. Steps 22 and 25 to 31 print the SHA-256 of each file sent and saved | `tests/tor-network/two-node.sh`, `tests/tor_network.rs` (`a_transfer_goes_on_across_a_key_rotation_and_a_contact_made_again_inherits_nothing`, ignored without the network) |

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
  every record and every pair of presented and active card.
- The active key of a contact changes only by a proven authorized
  successor, a confirmed pending one, or a replacement for a requested
  contact, over random sequences of credential events (T-CRED-13).
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
in the session layer (section 3a). Of the cases that need the core,
Phase 4 covers the flood of contact requests from many identities
(T-CONTACT-1, T-INV-4 and 5), strangers that stall after authentication
(the budget for strangers and its eviction, `tests/link.rs`) and a stale
endpoint epoch (`tests/credentials.rs`); the cases that need the message
queue and file transfer come with them.

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
| T-ORACLE-6 | With the budget for strangers exhausted, the non-contact classes are treated identically | core, Phase 4 (`tests/observability.rs`: no record, blocked, declined, deleted, a stale card and a pending key each evict the oldest silent stranger and see the same) |
| T-ORACLE-7 | Blocking or deleting an accepted contact during a session: the peer sees Close, then the behavior of the no-record class | first half in the session logic, Phase 1; the later sessions in core, Phase 4 (`tests/observability.rs`) |
| T-ORACLE-8 | On the wire: frame counts and byte counts are equal across the non-contact classes. Response times are recorded and compared only for gross differences | bytes: session layer, Phase 2, where everything one side writes is identical for no record, declined, blocked, and a contact with a stale or conflicting card; through the store over the mock transport, the same messages for every class (T-ORACLE-6 and 7), Phase 4; times: not measured yet |

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
6. Rates (since Phase 7): each message rate of a contact gives its burst
   and then one per interval, and another contact sending meanwhile is
   not slowed; the rate holds across the sessions of a contact; a session
   above `CONTACT_SESSION_RATE` is closed before it is confirmed; a
   session waiting for a token ends when it is closed (`tests/runtime.rs`,
   `rates::tests`). A property test runs any sequence of bursts of
   messages and sessions of three contacts, and records that end, against
   a token bucket written apart: every take is the model's, and a refusal
   names the first instant the model gives a token (P7-22).

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

Before the Whonix adapter, with one Gateway and two Workstations (W1 runs
Monolith, W2 is the attacker), on VirtualBox or KVM, and separately on
Qubes-Whonix:

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

Since Phase 4 the layer is `MemoryDir` in the storage crate: it counts the
steps of the directory, stops at any of them, and on restart keeps or
drops what was not flushed and keeps or tears what was written but not
flushed, in every combination. Covered: the vault's own write and
creation at every step (`vault::tests`), and, through the contact store
(`tests/store.rs`), creating an identity with its Onion Service key,
creating a contact, updating a card, promoting a successor, an announced
successor, blocking, deleting, creating and revoking an invitation and
beginning a rotation, each stopped at every step: afterwards the vault
opens and holds the state before or after the change, never a mixture,
and never a state something was granted on that is lost. Queue updates
come with the message queue (T-MSG-3). File completion comes with file
transfer: a crash of the receiver at every step of receiving, of the
sender at every step of an offer, of the receiver at every step of a
decision and of the sender at every step of a cancellation; the sweep of
receiving runs under every
crash outcome, the others under the first (T-FT-CRASH-RECV,
T-FT-CRASH-SEND, T-FT-MODEL).

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
| 3 | T-SOCKS, T-CTRL, T-CRED, T-LIFE pass; T-NET-1 passes; two-node chat over a private Tor network |
| 4 | T-CONTACT, T-ID, T-DUP, T-CONFIRM, T-ORACLE-6 to 8, T-CRASH, T-INJ pass. As Phase 4 was briefed (the contact store, persistence, lifecycle and credential convergence; `DESIGN_QUESTIONS.md` section 11), it adds T-INV, T-MI, the private network test over vaults and the mutation list of Phase 4, and leaves to the session manager and the queue before Phase 5: T-DUP-4 to 6, T-CONFIRM-1 timing, T-ORACLE-8 times, T-INJ for history and files, and T-CRASH for queue updates and file completion |
| 5 | T-FILE passes; transfer fuzzing clean |
| 6, 7 | platform matrix passes on the current release |
| 9 | T-RES passes with measurements recorded |
