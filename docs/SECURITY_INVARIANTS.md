# Security invariants

These statements must hold in every build, on every platform, against every
peer. A change that weakens one of them is a security change and needs the
review described in `docs/ARCHITECTURE.md` section 9.

The invariants are grouped by topic, so the numbers are not in order.
S29 to S37 were added after the first list was written.

Implemented so far. In the protocol core: the frame and field bounds of
S10, S11 and S26, the state gate of S19, the single encoding of S30, the
text and filename rules behind S14, S15 and S20, the message logic of S7,
S23 and S24, the card check of S33 and the stale-card rule of S36. In the
session layer: the pinning of S9, the handshake bounds of S10, the
randomness of S17, the redaction of S18 for its own types, the typed
session of S19 and S21, and S34, S35 and S37. Everything that involves
the network, storage or a user interface is still a planned mechanism.

Each invariant names the mechanism that enforces it and the tests that check
it. "Mechanism" means a structural property of the code (a type, a single
choke point, a missing capability), not a convention that reviewers have to
remember. Where Phase 0 can only plan the mechanism, the entry says so.

Test area names refer to `docs/TEST_PLAN.md`.

## Network

### S1. No connection silently falls back to clearnet

- Mechanism: `monolith-core` and `monolith-protocol` have no socket code and
  no dependency that can open a socket. The only way to obtain a peer stream
  is `TorBackend::connect_onion` or `OnionService::accept`. `SystemTorBackend`
  opens exactly two kinds of connection: to the configured SOCKS endpoint and
  to the configured control endpoint. Both are loopback or the configured
  Whonix-Gateway address. A failed SOCKS connection is an error, never a
  trigger for another transport.
- Tests: T-NET-1 (network namespace with only the Tor proxy reachable),
  T-NET-2 (SOCKS endpoint down: every operation fails, no other packet
  leaves), dependency check that only `monolith-tor` links a networking
  crate.

### S2. All peer connections go through a Tor backend

- Mechanism: same choke point as S1. `connect_onion` takes an
  `OnionServiceKey`, not a hostname or an IP address, so there is no value a
  caller could pass that names a clearnet host.
- Tests: T-NET-1, API review of `TorBackend`.

### S3. Onion names are never resolved by the operating system

- Mechanism: the backend builds the `.onion` name from the 32-byte key and
  sends it inside the SOCKS5 request as a domain name (address type 0x03).
  No Monolith crate calls a resolver API. `std::net::ToSocketAddrs` on a
  string, `getaddrinfo` wrappers and resolver crates are banned by
  `clippy::disallowed_methods` and `deny.toml` once the backend exists.
- Tests: T-NET-1 (no DNS packet observed), T-SOCKS-1 (request bytes carry
  address type 0x03).

### S4. Tails and Whonix builds never launch a second Tor

- Mechanism: no Monolith crate contains code to start a Tor process, and no
  backend embeds one. `SystemTorBackend` can only connect to an existing Tor.
  A future `ArtiBackend` is a separate crate behind a cargo feature that
  distribution builds for Tails and Whonix do not enable, and it refuses to
  start when platform detection reports Tails or Whonix.
- Tests: T-PLAT-TAILS and T-PLAT-WHONIX matrices (process list contains one
  tor), build check that the Tails and Whonix packages do not contain the
  Arti feature.

### S5. Monolith does not change Tor's anonymity configuration

- Mechanism: the control client can send only the commands listed in
  `docs/TOR_CONTROL_SURFACE.md` section 1. Commands are values of a closed
  enum; there is no "send raw command" API. Section 4 of that document
  lists what is never sent, including `SETCONF` and `SIGNAL`.
- Tests: T-CTRL-1 (recording mock control port: only allowlisted commands
  are ever sent), fuzzing of the command serializer for line injection.

### S6. The Tor control interface is least-privilege

- Mechanism: the allowlist in `docs/TOR_CONTROL_SURFACE.md` is the complete
  set of commands Monolith can emit. The onion-grater profiles in
  `integrations/` add only that set. On Tails the Monolith profile is the
  whole filter for the Monolith process, so the restriction is also
  enforced from outside. On Whonix the Gateway applies the union of its
  default profile and every enabled profile to all Workstations; the limit
  enforced from outside is that union, not Monolith's set alone.
- Tests: T-CTRL-1, T-PLAT-WHONIX (commands outside the profile are refused
  by the Gateway).

### S8. A peer never makes Monolith connect to a third party on unauthenticated input

- Mechanism: outbound connections are made only to the endpoint in a contact
  record. A contact record is created from a contact card the user imported,
  or from a contact request the user accepted. Its endpoint changes only
  through a binding signed by the pinned identity with a greater epoch. No
  protocol message carries an address that is dialed as a side effect of
  receiving it. There is no back-connection step in the handshake.
- Residual: a contact can sign a binding that names an Onion Service it does
  not control. Monolith will then dial that service; the handshake fails
  because the service cannot prove the contact's identity. See THREAT_MODEL
  section 4, adversary B.
- Tests: T-PROTO-STATE (no message type triggers a dial), T-ENDPOINT-1.

## Identity and contacts

### S7. Unknown peers never modify the permanent contact database

- Mechanism: the only write path that creates a contact is the `AcceptRequest`
  and `AddContactCard` commands, both issued by the local user. A session
  with an identity the user has not added has write access to one thing: a
  bounded in-memory pending-request queue. For an identity the user did
  add, the peer's ContactAccept or ContactRequest can complete the
  acceptance the user started; it cannot create a record.
- Tests: T-CONTACT-1 (request flood leaves the contact store byte-identical),
  T-PROTO-STATE.

### S9. Identity changes for an existing contact are never accepted silently

- Mechanism: an outbound handshake has one input that names the peer, the
  pinned card. `HandshakeInitiator::start` puts the identity key of that
  card into the prologue and its transport key into the Noise pre-message.
  An endpoint that does not hold that transport key, or that is another
  identity by its own account, cannot produce a message 2 that verifies.
  The initiator then stops with `IdentityMismatch`. It has sent 48 bytes
  that carry nothing of its identity, and it sends nothing more. There is
  no function that continues such a handshake and no parameter that makes
  the check optional. The session logic holds the dialed identity and
  refuses to be authenticated as any other, so a session never turns into
  one with an unknown or a different peer. There is no API that replaces a
  pinned identity key; the user must delete the contact and add the new
  identity as a new contact.
- Tests: T-ID-1 (mismatch is a hard failure), T-ID-2 (no code path updates a
  pinned key), T-HS-PIN (`tests::handshake`: an endpoint without the
  transport key, a second message from another handshake, a card that
  names another party's transport key).

### S33. The three keys of an identity are never the same key

- Mechanism: key separation. The identity key, the transport key and the
  Onion Service key are generated independently and held in different
  types (`IdentityPublicKey`, `TransportPublicKey`, `OnionServiceKey`).
  The identity key signs contact cards and nothing else; no session code
  holds it. A contact card in which an endpoint is byte for byte the
  identity key, or in which the transport key is the Montgomery form of
  the identity key or of an endpoint key, is rejected by
  `ContactCard::decode`, and `ContactCard::sign` returns an error instead
  of signing one, so the protocol never carries a statement that one key
  serves two domains. The check catches reuse that a card can show; it
  cannot catch keys derived from one secret by other means.
- Tests: T-CARD (validly signed cards with each coincidence; signing such
  cards), fuzz target `contact_card`.

### S36. A card that is older than the newest one held, or contradicts it, never opens a contact session

- Mechanism: the stale-card rule. The standing of a peer comes from one
  function, `PeerRecord::admit`, which takes the local record with the
  newest card held of the identity and the card that stands for the peer
  on the session: the one presented in the handshake, or the one that was
  dialed. A lower epoch, or the same epoch with another transport key or
  endpoint set, gives the standing `StaleCard`, which the session logic
  treats on the same code path as an identity that is not a contact.
  Neither `InboundPeer` nor `OutboundPeer` can be turned into a session
  without that function. The newest card held is the pinned one or a
  later one that the user has not confirmed: a successor card counts from
  the moment it was received. The same comparison, `evaluate_card`,
  decides what an EndpointUpdate means; nothing but a greater epoch
  changes what is held.
- Residual: a party with no record of the identity, or one that has
  received only the older card, has nothing to compare with. Keeping the
  newest card and handing it to `admit` is the job of the contact store,
  which does not exist yet.
- Tests: T-STALE (`session::tests`, `tests::contacts`: a retired transport
  key against a contact that has received the new card, confirmed or not;
  a dialed card that was superseded meanwhile; the property test of the
  standing table).

### S20. The display name is never a security identifier

- Mechanism: contacts are keyed by identity public key in every map, table
  and message. Display names are stored as opaque validated text, byte for
  byte as received, and are not unique. They are not normalized, and no
  decision about identity, authentication, contact equality, authorization,
  duplicate detection or protocol state reads one. A front end may
  normalize a copy for drawing or searching. Security-relevant UI shows the
  fingerprint next to the name.
- Tests: T-ID-3 (two contacts with the same name stay distinct), T-TEXT-*
  (names that differ only in normalization are both accepted and stay
  different).

### S21. Connection state is tied to the cryptographic identity

- Mechanism: the session table is keyed by the identity key the handshake
  established. A session has no entry in it before the handshake is
  complete and the card checks have passed. `AuthenticatedSession` can be
  obtained only from a completed handshake, holds the card that stands for
  the peer for its whole life, and has no function that changes it. A
  card inside a message that does not belong to that peer is a violation:
  another identity always, and in a ContactRequest also another card than
  the one of the handshake.
- Tests: T-DUP-*, `session::tests` (authentication before the handshake is
  complete, as the local identity, as another identity than the dialed
  one; foreign cards in ContactRequest and EndpointUpdate for every
  standing), `tests::frames` (a card of another identity on a real
  session).

### S22. Duplicate-connection handling happens only after authentication

- Mechanism: duplicate resolution reads the session table of S21, which
  contains authenticated sessions only. An unauthenticated stream cannot be
  compared with, replace or close any other session.
- Tests: T-DUP-1 to T-DUP-6.

### S23. Blocked and unknown peers learn nothing about the user's contacts

- Mechanism: in `AuthenticatedUnknown` Monolith sends only ContactAccept to
  a peer it holds as an accepted contact, ContactRequest to a peer the user
  has requested, and Close. To any other peer it sends only Close. A first
  message from a blocked, declined, already pending or capability-less
  identity, from a contact that presented a stale card, or to a full
  queue, is answered the same way in every case: Close, with no other
  reply. The handshake before it does not look at any record of the peer.
- Tests: T-CONFIRM-1, T-ORACLE-1 to T-ORACLE-8 (the cases cannot be told
  apart by what is sent; equal timing is not claimed), T-BLOCK-1,
  `tests::contacts` (what one side writes is byte for byte the same in
  every such case).

### S24. No protocol operation answers questions about other peers

- Mechanism: no message type has a field that names a third party. Every
  identity or endpoint that appears on the wire belongs to the sender and is
  signed by the sender.
- Tests: review of PROTOCOL.md section 8 on every protocol change;
  T-CONFIRM-2.

## Protocol and parsing

### S10. Every network read has a strict upper bound before allocation

- Mechanism: the three handshake messages have fixed sizes, 48, 48 and 235
  bytes, and the handshake functions take them as arrays of exactly that
  size; `MessageBuffer` collects exactly that many bytes from the stream
  and not one more. A transport frame is read by first reading its 2-byte length,
  rejecting values outside the range allowed in the current state, and then
  filling a buffer for exactly that length. The length was checked first,
  so the buffer is never larger than the limit of the state: one padding
  block before a session is confirmed, one maximum frame after. No other
  allocation size comes from peer input.
- Tests: T-FRAME-1 to T-FRAME-6, fuzz targets `frame_stream`,
  `frame_plaintext`, `handshake_responder`, `handshake_initiator` and
  `session_frames`.

### S11. Every protocol field has an explicit maximum

- Mechanism: PROTOCOL.md gives a maximum for every field. The decoder takes
  the maximum as a required argument for every variable-size read; there is
  no unbounded read function. All maxima live in
  `monolith_protocol::limits`.
- Tests: T-FIELD-* (max and max+1 for every field), property tests.

### S19. Application data is accepted only in an authenticated state

- Mechanism: before a peer is authenticated there is nothing that could
  take a frame. The handshake types have no function for frames; only
  `AuthenticatedSession` has, and only a completed handshake yields one.
  On a session, `MessageType::may_be_received_in(SessionState)` is
  evaluated for every frame before its body is parsed, and again by
  `Session::receive`, which is the only producer of the `Deliver` action
  and produces it in `AuthenticatedContact` alone. In the other direction
  `AuthenticatedSession::send` encrypts a message only if the session
  logic allows it in the current state. The application-side handler type
  that can only be constructed for a confirmed session comes with
  `monolith-core`.
- Tests: `message::tests`, T-PROTO-STATE (arbitrary event sequences in the
  property tests and in the `session_sequence` fuzz target), `tests::frames`
  (a chat message before confirmation, sent and received).

### S26. The parser never buffers an attacker-controlled amount of data

- Mechanism: the largest unit the parser holds is one frame (S10). No
  message spans frames. File data is written to disk chunk by chunk; chat
  text fits in one frame.
- Tests: T-RES-1 (memory high-water mark under hostile input), fuzzing with
  an allocation limit.

### S27. Malformed input causes bounded failure

- Mechanism: decoders return `ProtocolError`; the workspace lints forbid
  `unwrap`, `expect`, `panic!`, unchecked indexing and unchecked arithmetic
  in non-test code. Each session runs in its own supervised task, so a bug
  that still panics ends one session, is counted, and does not take the
  process down. State is written only after validation (S28).
- Tests: all fuzz targets, T-MAL-*.

### S30. Signed structures have exactly one valid encoding

- Mechanism: contact cards are fixed-layout byte
  strings with no optional ordering, no maps and no variable-width integers.
  The decoder accepts exactly one encoding of each value and rejects
  trailing bytes. The bytes that are signed are produced by a dedicated
  function, separate from the transport encoder, so a change to the
  transport layout cannot change what a signature covers.
- Tests: T-CARD-* (bit flips, trailing bytes, non-canonical base32),
  property tests.

## Resources

### S12. All queues and channels are bounded

- Mechanism: every queue capacity is a constant in `limits`. When a channel
  crate becomes a dependency, `clippy::disallowed_methods` will ban its
  unbounded constructors, starting with
  `tokio::sync::mpsc::unbounded_channel`. The lint entry does not exist
  yet because the path it names does not.
- Tests: lint in CI, T-RES-3.

### S13. File transfer is always offered and explicitly accepted

- Mechanism: a FileChunk is valid only for a transfer identifier that is in
  the `Accepted` state, and that state is entered only by a user command.
  A chunk for a transfer that was never accepted is a violation and closes
  the session. A chunk for a transfer that has already ended is discarded.
- Tests: T-FILE-1, T-FILE-2.

## Data handling

### S14. Remote filenames are data, never filesystem paths

- Mechanism: the received filename is validated, stored as a display
  string, and never passed to a filesystem call. Incoming data is written to
  a file whose name Monolith generates. The final name is chosen by the user
  in a save dialog (or derived from the sanitized display name inside a
  directory the user chose) and created with exclusive-create semantics.
- Tests: T-FILE-NAME-*, fuzz target `text_fields`.

### S15. Remote text is data, never markup or code

- Mechanism: text fields are validated UTF-8 with control characters
  rejected at the protocol layer. Front ends render them with plain-text
  widgets; no markup parser is linked. The CLI escapes control and
  non-printing characters before writing to a terminal.
- Tests: T-TEXT-*, T-INJ-*.

### S16. No received URL, file or resource is opened automatically

- Mechanism: no Monolith crate depends on an HTTP client, an image decoder,
  a URL opener or a file-type sniffer. Front ends offer "copy" for links.
- Tests: ban list in `deny.toml`, T-NET-1.

### S29. Displaying a message causes no network or filesystem access

- Mechanism: follows from S15 and S16; rendering is a pure function of the
  validated text.
- Tests: T-NET-1 while rendering hostile messages.

### S28. Unvalidated network input is never persisted as configuration

- Mechanism: storage accepts typed records only. Every record type that can
  hold peer-supplied text is built by a validating constructor. There is no
  text configuration file whose lines are assembled from field values.
- Tests: T-INJ-* (newline, delimiter, SQL, path and escape-sequence
  injection through every text field).

## Cryptography and secrets

### S17. Cryptographic randomness comes only from a CSPRNG

- Mechanism: the operating system source is read through `getrandom` in
  one crate, `monolith-session`, in two places: `TransportSecretKey::generate`
  and the random source that the resolver hands to the Noise library for
  the ephemeral keys of a handshake. Nothing else in the workspace draws
  randomness, and no other generator crate is a dependency of a product
  build. Non-cryptographic generator crates are banned by `deny.toml`. A
  failure of the source fails the operation; there is no fallback. A
  handshake with fixed ephemeral keys can be built only in the tests of
  that crate and in a build made with `--cfg fuzzing`; no cargo feature
  enables it, so it cannot be switched on through a dependency.
- Tests: `resolver::tests` (the source fills its buffer, two keys differ,
  a failing source leaves no key), `tests::handshake` (two handshakes
  differ in every message), T-RNG-1.

### S18. Secrets and sensitive values do not appear in ordinary logs

- Mechanism: types that hold secrets, message content, onion addresses,
  identity keys, fingerprints, filenames, capabilities and isolation tokens
  do not derive `Debug` and do not implement `Display`. Their hand-written
  `Debug` prints `[redacted]`, or sizes and a type name where that is all
  there is to say. A type that is made only of such types may derive
  `Debug`, because the fields print themselves. Error types carry no peer
  data. See `monolith_identity::redact`. The handshake and session types
  print `[redacted]`; a session prints its state and its frame counters.
  The session crate has no logging at all and returns no key material from
  any function.
- Tests: unit tests per type, T-LOG-1 (run the malicious-peer suite with
  logging at maximum verbosity and search the output for every secret and
  every peer-supplied string used by the suite).

### S34. Session keys and ephemeral keys never leave the session objects

- Mechanism: the ephemeral private key, the chaining key and the cipher
  keys of a session exist only inside the Noise state that a handshake or
  session object owns. No function of `monolith-session` returns them, no
  type that holds them can be serialized or cloned, and the crate has no
  storage and no logging. There is no resumption, no pre-shared key and no
  ticket, so nothing of one session is input to another. The transport
  private key is the only secret that outlives a session; it is held in a
  type that erases it on drop and is passed to the Noise library by
  reference.
- Residual: erasure is best effort, and the Noise library does not erase
  what it holds itself (`CRYPTOGRAPHY.md` section 8).
- Tests: review of the public API of `monolith-session`; `Debug` tests of
  every type.

### S35. A failure tells the peer nothing but that the stream closed

- Mechanism: the handshake functions return a message to send or an error,
  never both. `AuthenticatedSession::receive` returns an error and no
  frame. The protocol has no error message, and Close has an empty body
  and is not sent after a violation. What went wrong is a value for local
  use that carries no data from the peer; the caller closes the stream.
- Tests: T-ORACLE-4, `tests::handshake` and `tests::frames` (every failure
  returns an error and nothing to write), fuzz targets `handshake_responder`
  and `session_frames`.

### S37. A session ends at its limits, and the limits can only be lowered

- Mechanism: `AuthenticatedSession` counts frames and ciphertext bytes in
  each direction and knows when it was established. `send` refuses every
  message but Close once the age, frame or byte limit is reached, and
  always leaves room for one Close. `receive` treats a frame beyond a limit
  as a violation. `SessionLimits::new` returns nothing for a value above
  the protocol's. There is no rekey and no way to reset a counter; a new
  session is a new handshake.
- Tests: T-LIMIT (`tests::frames` with reduced limits, the property test
  that a session never sends more than its limits).

## Platform

### S25. Tails persistence is opt-in and explicit

- Mechanism: on Tails the default state mode is `Ephemeral`. A persistent
  vault is created only by a user command that names a location, and only
  inside the Persistent Storage mount. Monolith writes nothing to disk in
  ephemeral mode.
- Tests: T-PLAT-TAILS (filesystem diff after an ephemeral session).

### S31. Critical state changes are atomic

- Mechanism: the vault is replaced by write-to-temporary, fsync, rename,
  fsync-directory. The identity key is never modified in place; every write
  produces a complete new file that carries it.
- Tests: T-CRASH-* (kill at every write step).

### S32. No hidden network traffic

- Mechanism: Monolith has no update check, telemetry, crash reporting or
  time synchronization. The only traffic it causes is peer sessions and the
  Tor control and SOCKS exchanges needed for them.
- Tests: T-NET-1 (idle run produces no traffic besides Tor's own).
