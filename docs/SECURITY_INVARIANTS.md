# Security invariants

These statements must hold in every build, on every platform, against every
peer. A change that weakens one of them is a security change and needs the
review described in `docs/ARCHITECTURE.md` section 9.

The invariants are grouped by topic, so the numbers are not in order.
S29 to S52 were added after the first list was written.

Implemented so far. In the protocol core: the frame and field bounds of
S10, S11 and S26, the state gate of S19, the single encoding of S30, the
text and filename rules behind S14, S15 and S20, the message logic of S7,
S23 and S24, the card check of S33, the credential rules of S36 and S48
and the part of S38 that cards and sessions hold. In the session layer:
the pinning of S9, the handshake bounds of S10, the randomness of S17, the
redaction of S18 for its own types, the typed session of S19 and S21, S34,
S35 and S37, and the local party of S47. In the Tor adapter and the core,
since Phase 3: S1 to S3 and S5, the command set of S6, the connection
budgets of S12, the withdrawal of S49 and the admission of S50 up to the
contact store, and the parts of S42, S43, S45 and S46 named in their
entries. Since Phase 4, which waits for its final verification, the
contact store and the vault: S7, S28 and S31 for the vault, S36 and S48
to S51 through the store, S38 and S39 to S46, and S17 for the new users
of the random source. Everything that involves a user interface, the
message store or file transfer is still a planned mechanism.

Each invariant names the mechanism that enforces it and the tests that check
it. "Mechanism" means a structural property of the code (a type, a single
choke point, a missing capability), not a convention that reviewers have to
remember. Where Phase 0 can only plan the mechanism, the entry says so.

Test area names refer to `docs/TEST_PLAN.md`.

## Network

### S1. No connection silently falls back to clearnet

- Mechanism: only `monolith-tor` contains socket code. The standard
  library and tokio could open sockets anywhere (Cargo unifies tokio's
  `net` feature across the workspace), so `clippy.toml` bans the socket
  constructors that take an address argument, and every UDP socket type,
  in every crate; the two uses in `monolith-tor`, the SOCKS and control
  connection and the loopback listener, pass a `SocketAddr` and are
  allowed where they are. The only way to obtain a peer stream
  is `TorBackend::connect_onion` or `OnionService::accept`. `SystemTorBackend`
  opens exactly two kinds of connection: to the configured SOCKS endpoint and
  to the configured control endpoint, and binds one listener per
  published service, on 127.0.0.1. An `Endpoint` is a loopback address or an absolute socket
  path; anything else is refused when the configuration is read (the
  Whonix-Gateway address belongs to the platform adapter of Phase 7).
  `Endpoint` is opaque: only its validating constructors make one, and the
  connection code checks the rule again. A failed SOCKS connection is an
  error, never a trigger for another transport.
- Tests: T-NET-1 (`tests/network/fail-closed.sh`: every network system
  call of the onion connection paths and of `monolith tor status`, with
  SOCKS present, absent, and in a namespace with only loopback, goes to a
  configured loopback endpoint), the backend tests against scripted
  servers (`monolith-tor/tests/system.rs`: no SOCKS endpoint, no
  connection; an onion failure is an error), the core tests on the mock
  (a dial without SOCKS reaches nothing), `config::tests` (non-loopback
  endpoints refused). Only `monolith-tor` enables tokio's `net` feature
  and contains socket code.

### S2. All peer connections go through a Tor backend

- Mechanism: same choke point as S1. `connect_onion` takes an
  `OnionServiceKey`, not a hostname or an IP address, so there is no value a
  caller could pass that names a clearnet host.
- Tests: T-NET-1, API review of `TorBackend`: no function of the trait
  takes a host name, an address or a port; the port is
  `ONION_VIRTUAL_PORT`.

### S3. Onion names are never resolved by the operating system

- Mechanism: the backend builds the `.onion` name from the 32-byte key and
  sends it inside the SOCKS5 request as a domain name (address type 0x03).
  No Monolith crate calls a resolver API. `ToSocketAddrs::to_socket_addrs`
  and `tokio::net::lookup_host` are banned by `clippy.toml`; endpoints are
  `SocketAddr` values or paths, so the socket calls never see a name.
- Tests: T-NET-1 (no UDP socket and nothing naming port 53 in the system
  call trace, also in a namespace without DNS), T-SOCKS-1 (`socks::tests`
  and `system.rs`: the request carries address type 0x03 and the literal
  `.onion` name, which a local resolution could not have produced).

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
- Tests: T-CTRL-1 (`system.rs`: a recording control server sees only the
  allowlisted lines, byte for byte), `command::tests` (the exact bytes of
  every command; no forbidden flag in either `ADD_ONION` form), fuzz
  target `tor_control_reply` (64 arbitrary key bytes never add a line).

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

- Mechanism: the contact store of each local identity creates a record
  in two operations, both the local user's: importing a card
  (`LocalIdentity::import`) and accepting a pending request
  (`accept_request`). A session with an identity the user has not added
  has write access to one thing: the bounded in-memory queue of pending
  requests (`Requests`), which is never written to the vault. For an
  identity the user did add, the peer's ContactAccept or ContactRequest
  can complete the acceptance the user started (`MarkAccepted`, applied
  by `LocalIdentity::apply` under the lock of the contact and only while
  the session stands); it cannot create a record.
- Tests: T-CONTACT-1 (request flood leaves the contact store
  byte-identical), T-PROTO-STATE, `tests/credentials.rs` in the core
  (`a_stranger_request_reaches_the_queue_and_nothing_durable`: the vault
  is unchanged), `tests/invitations.rs`.

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

### S36. A card of another key that is older than the active credential, a card that contradicts it, and the retired key never open a contact session

- Mechanism: the stale-card rule, half of rule F4. Every card of a known
  identity is judged by one function, `contact::decide`, from the kind of
  record, the credentials and the reason the card is looked at: the card
  presented in a handshake or the one that was dialed, an EndpointUpdate,
  an import, a confirmation. `Credentials::relation` classifies the card
  the same way whatever its source: a card of another key older than the
  active card or than the authorized successor, a card with the epoch of
  the active card or of the authorized successor that states something
  else, a card of the authorized key older than the announced one, or the
  retired key is stale or a conflict, and gives `StaleCard`, which the
  session logic treats on the same code path as an identity that is not a
  contact. Only the key retired last is remembered; a key retired earlier
  is refused in its old cards by their epochs, and a newer card stating
  it is a new key without continuity (S48). The contact store holds the
  credentials and calls `decide` under the lock of the contact
  (`PeerRecord::admit` and `import` call it too), and neither
  `InboundPeer` nor `OutboundPeer` can be turned into a session without
  it. The credentials are stored in the vault and restored as they were,
  the retired key included, so a restart does not forget it.
- Residual: a party with no record of the identity, or one that never
  promoted the successor, has nothing newer to compare with.
- Tests: T-STALE (`session::tests`, `credential::tests`,
  `tests::contacts`: a retired transport key, also with a higher epoch,
  against a contact that promoted the new one; a dialed card whose key
  was retired meanwhile; the property tests of the standing table and of
  the credential transitions), `credential::tests::table` (every relation
  from every source against a model written from `PROTOCOL.md` section
  11.4), `contact::tests`, `tests/store.rs` in the core (the retired key
  after a restart), fuzz targets `credential_sequence` and
  `contact_store`, mutation faults CS1 to CS6.

### S47. A local party belongs to one local identity

- Mechanism: rule F2, half B. `LocalParty::issue` is the only way to make
  a party: it signs the card from the identity key that is passed in, for
  the transport key that is passed in, and decodes the result as a
  receiver would. There is no constructor that takes a card, so a card
  from outside, one signed by another identity and naming the local
  transport key included, cannot become the local card. The identity key
  is borrowed for the signature and not kept.
- Tests: `key::tests` (the party's card is signed by its own identity key
  and states its own transport key; key separation is checked), the
  `compile_fail` example on `LocalParty` (adopting a card does not
  compile), `tests::handshake` (a party acts only as the identity that
  signed its card, as responder and as initiator), mutation faults K2 to
  K4.

### S48. A newer transport key never replaces the active one without continuity or the user

- Mechanism: rule F4. `Credentials` is the only holder of which key stands
  for a contact. A newer card with another key becomes the authorized
  successor only through `Credentials::announce` on a session that was
  authenticated with the active key, and the active key only when a
  handshake proves it (`Credentials::admit`). Any other newer key is
  pending: the session gets `Standing::PendingSuccessor`, which the
  session logic treats as an identity that is not a contact, and the key
  takes over only through `Credentials::confirm` with exactly the card
  that is pending. A card the user imported is never displaced from the
  pending slot by a presented one. An import for an accepted contact
  (`Credentials::import`) never replaces the key. Every transition that replaces the key retires
  the previous one. A copied identity key alone therefore cannot take a
  contact over or lock the holder of the active key out. Since Phase 4
  every entry point applies one table (`Credentials::evaluate`), so an
  import and an admission cannot judge one card two ways, and the store
  applies the change under the lock of the contact and makes it durable
  before anything that depends on it is used. On the side that rotates,
  the successor is announced, and the new key answers, dials and is
  handed out, only once that step is in the vault, so a crash never
  leaves a contact with a key the identity no longer holds.
- Residual: a holder of the identity key and the active transport key can
  produce continuity and is indistinguishable from the identity
  (`THREAT_MODEL.md` adversary Q).
- Tests: `credential::tests` (the transitions of the review, simultaneous
  rotation, the property test that the key changes only through
  continuity or the user, the exhaustive table), `tests::contacts`,
  `tests::credentials`, `tests/store.rs` in the core
  (`promoting_a_successor_is_atomic`, `an_announced_successor_is_atomic`),
  `tests/credentials.rs` (`the_new_key_reaches_no_peer_before_it_is_durable`,
  `a_rotation_ends_only_after_a_durable_switch`), mutation faults CR1 to
  CR9, CS1 to CS6, CS49, CS50 and CS54.

### S49. A session of a retired transport key delivers nothing after the retirement

- Mechanism: `Credentials::authorizes` says whether the card of a session
  still states the active key. When a key is retired, every session that
  was admitted as a contact's and for which it turns false is withdrawn
  before anything else is done with the contact:
  `AuthenticatedSession::withdraw` drops its standing and ends it with
  Close, and nothing it receives afterwards is delivered. Each link has a
  `Withdrawal`, which the admission function keeps with the session in
  the same step as the admission and which is handed out nowhere else;
  withdrawing it stops delivery before the next frame is taken from the
  buffer, wakes a link that waits for the peer, ends a write in progress
  without a Close, and makes every later call on the link fail at once.
  Deleting or blocking the record of an identity withdraws its sessions
  the same way. `Withdrawal::is_ended` tells the store which links are
  gone, including those whose admission failed. Sessions admitted with another standing are left
  on the path of a stranger, so that their end does not show the peer
  that it is held as a contact. The duplicate rule takes for each session
  whether its key is current (`duplicate::Contender`), and a session that
  is not loses before the preference is looked at. The contact store
  keeps the withdrawal of every session admitted as a contact's in the
  slot of that contact, and after every change of the slot withdraws
  each one that no longer stands, in the same step: a promotion, a
  block, a deletion, a confirmation, any later transition. A failed
  write of the vault withdraws every session of the installation. A
  dial or an answer that is cancelled while its admission is made
  durable ends its withdrawal, so the store does not keep the session.
- Residual: a message the link returned before the withdrawal is the
  caller's to judge: what it would change in the contact state is
  applied by `LocalIdentity::apply` under the lock of the contact and only
  while the session still stands (`ARCHITECTURE.md` section 1.2).
- Tests: `session::tests` (a withdrawn session delivers nothing),
  `duplicate::tests`, `tests::credentials` (the session of the old key
  after a promotion; the duplicate rule), `tests/credentials.rs` in the
  core (a link with a buffered and an unread message, a waiting link, a
  link withdrawn before it was polled, a withdrawal seen first by a send;
  a stale session is not withdrawn; ended links; a failed admission;
  `blocking_or_deleting_a_contact_withdraws_its_open_links`,
  `a_failed_installation_admits_nobody`), `tests/store.rs`
  (`a_block_that_races_an_admission_leaves_no_contact_session_standing`),
  the unit tests of `link` (a withdrawal during a pending send), the
  private network test (steps 6 to 8), `tests/credentials.rs`
  (`a_dial_cancelled_while_its_admission_is_made_durable_leaves_no_session`),
  mutation faults CR10 to CR17, CR34, CS8 to CS10 and CS53.

### S50. A session is admitted against the contact state of that moment

- Mechanism: `link::dial` and `link::answer` hold no contact state. They
  take the local identity and admit the peer through its contact store
  once, when the peer is authenticated: the dial budget, the Tor stream
  and the handshake messages that authenticate the peer are behind it.
  The store takes the lock of the contact, looks the record up as it is
  then, decides, applies the change of the credentials and keeps the
  `Withdrawal` of the link in that one step, and nothing in the step
  waits. A retirement after that point withdraws the session (S49).
- Tests: `tests/credentials.rs` in the core (a key retired while a dial
  is in progress gives no contact session; a key retired while the
  admission is made durable), `tests/store.rs` (concurrent operations on
  one contact are one of their serial orders), `tests::contacts`,
  mutation faults H16, CS11.

### S51. The local identity goes in message 3 only to a key that may learn it

- Mechanism: a dial admits the responder after message 2 and writes
  message 3, which carries the local identity and card, only if
  `Admission::may_learn_local_identity` holds for the standing of the
  session the admission function returned, not for the admission it
  returned beside it: the proven key stands, as of that moment, for an
  identity held as a requested or accepted contact. That is decided on
  the transport key and the credentials, not on a card object: an older
  card of the active key qualifies; a pending or retired key, a key
  older than the announced successor, a contradicting card, and a
  deleted, declined or blocked identity do not. The write races the
  withdrawal of the session: a withdrawal before or during it ends the
  dial and the session. A refused dial returns `LinkError::Refused` with
  the admission, so a conflict stays visible to the local side; the peer
  sees the stream close, as for a failed handshake.
  Since Phase 4 message 3 does not exist before that decision:
  `HandshakeInitiator::read_message_2` returns the authenticated
  responder (`OutboundPeer`), and `OutboundPeer::admit` makes message 3
  only for a standing that may learn the local identity. The contact
  store is the only production caller of `admit`, and before `link::dial`
  writes message 3 the state the admission depended on is durable.
- Residual: bytes the stream accepted before a withdrawal cannot be
  called back. The first 48 bytes of message 3 carry the local transport
  key, which identifies the local side to a responder that knows its
  card. A local stream takes the 235 bytes in one write in practice.
- Consequence: an outbound session is always a contact's, and none takes
  a slot of `MAX_UNKNOWN_SESSIONS`.
- Tests: `tests/credentials.rs` in the core (withdrawal right after the
  admission, an older card of the active key, a conflicting card, a
  record deleted, declined or blocked during the dial, a full budget for
  strangers, a promotion whose message 3 cannot be written, a pending key,
  a key older than the announced successor, an admission returned that
  contradicts the session, the conflict reported locally, a key retired
  while the admission is made durable), the unit tests of `link` (a
  withdrawal before and during a stalled write of message 3), the tests
  of `handshake` in the session crate (no message 3 for a standing that
  may not learn the local identity), `tests/structure.rs` in the core
  (only the store admits an outbound peer; `link::dial` writes the
  message 3 of a granted admission once), mutation faults CR18, CR22 to
  CR24, CR35, CR40, CS15 and CS16.

### S52. Every session ends at its deadlines and on every failure

- Mechanism: `AuthenticatedSession::deadline` gives the earliest deadline
  of a session: a begun frame within `FRAME_READ_TIMEOUT` of its first
  byte, the next complete frame within `IDLE_TIMEOUT`, the age limit, and
  in `AuthenticatedUnknown` `UNKNOWN_SESSION_TIMEOUT` and, for a peer
  that is not a contact, `UNKNOWN_FIRST_MESSAGE_TIMEOUT`; a Close the
  session logic decided on a message it received is due at once. Bytes
  of an incomplete frame move none of them. `Link::receive` waits for
  that moment alongside the stream and the withdrawal, so a deadline
  fires while bytes are awaited, and a message that ends the session,
  the first one of a stranger or a Close from the peer, ends the link
  before it is returned. The end of the stream, a read or write error, a
  deadline, a withdrawal, a violation and such a message all end the
  link through one path: the session is over, its slot for strangers
  goes back, the stream is shut down once, within its bound, and every
  later call, `close` included, fails or returns at once without waiting
  for the stream again. The slot of an inbound stranger follows the
  standing of the session, not the admission returned beside it.
- Tests: `tests::deadlines` in the session crate, the unit tests of
  `link` (a frame sent a byte at a time, a stranger that sends nothing or
  sends its first message, a Close from the peer, an unconfirmed
  session, read and write errors, later calls on a link that is over,
  a stream whose shutdown never completes),
  `tests/link.rs` in the core (the slot of a stranger whatever the
  admission says), fuzz target `session_frames` (partial frames, expiry
  at the deadline it works out itself), mutation faults CR16, CR25 to
  CR33 and CR39.

### S38. An invitation capability admits a request and does nothing else

- Mechanism: a capability is the input of one decision, whether a
  ContactRequest from a peer that is not a contact goes into the queue
  (PROTOCOL.md section 12, step 3), and that decision is taken after the
  Close. The handshake, `PeerRecord::admit` and `evaluate_card` do not
  read it, and nothing the receiver sends depends on it, so it cannot
  authenticate a peer and cannot change what a peer sees. The handshake
  only carries it from the card held of the peer into the session, for
  the request. Cards of one identity that differ only in their capability are
  one statement to `evaluate_card`. A session sends only the capability
  of the card it holds of the peer. Revoking a capability removes it from
  the active set and touches nothing else: no contact record, session,
  key or history refers to the capability a contact once used. The
  contact store of each identity holds its active set, at most
  `MAX_ACTIVE_INVITATIONS`, compares a capability with every member in
  constant time, and stores the set in the vault; a capability is durable
  before the card that carries it is returned, and a revocation is
  durable before it is reported.
- Tests: `card::tests` (cards that differ only in their capability are
  each valid and do not conflict; a capability is not part of what is
  pinned), `tests::contacts` (a capability changes nothing a requester can
  see; a request carries the capability of the card held of the peer and
  no other), T-ORACLE-1, T-INV-1 to T-INV-11 (`tests/invitations.rs` in
  the core), `tests/store.rs`
  (`creating_and_revoking_an_invitation_are_atomic`), the private
  network test (step 8), mutation faults CS18 to CS21.

## Local identities

An installation may hold several local identities (`ARCHITECTURE.md`
section 1.1). The mechanisms below are held by the contact store, the
installation and the vault of Phase 4, and by the Tor adapter of Phase 3.

### S39. Each local identity has its own identity, transport and Onion Service keys

- Mechanism: every key of an identity is generated from the CSPRNG for
  that identity alone and held only in its context; S33 separates the
  three keys within one identity. The installation refuses to create or
  restore an identity whose identity key, transport key, transport key in
  rotation or Onion Service key equals one held by another local
  identity, and a vault that holds two such identities is refused when it
  is opened, so one identity key is never held by two contexts (that
  would be a form of multi-device, which version 1 does not have). Tor
  refuses to publish a key it already holds, and so does the mock
  backend.
- Tests: T-MI-1 and T-MI-9 (`tests/identities.rs` in the core),
  `tests/store.rs` (`identities_restored_or_created_never_share_a_key`),
  mutation fault CS23.

### S40. Contact state belongs to one local identity

- Mechanism: every record of an accepted, requested, blocked, declined or
  former contact, a pending request, a verification mark or a local alias
  is keyed by the local identity and the remote identity. No store
  function takes a remote identity alone. Accepting, blocking or deleting
  a peer for one identity changes nothing for another. A block list for
  all identities, if one is ever offered, is a separate record and an
  explicit choice. Each `LocalIdentity` has its own contact store, and
  every operation is a method of one identity; `link::answer` and
  `link::dial` take the identity a link is for. The store that admits a
  session binds the session's withdrawal to itself in the admission step,
  and an identity applies what arrived on a session only if its own store
  admitted it: a session of A handed to B, or to an identity made again
  from A's keys, is refused (`StoreError::OtherIdentity`). The vault holds the
  records of each identity apart. Deleting an identity closes it before
  it leaves the installation: it refuses every change and admission, its
  waits for durability fail, it holds no Onion Service key, and its accept
  loop and supervisor end and remove its service; nothing it held is
  written again. It hands out nothing that holds or uses a secret: no
  party to dial or answer with (`answering_party` fails, `dial_plan` is
  `None`, so `dial` and `answer` fail before a stream or a handshake), no
  card signed for a capability, no capability, no Onion Service secret.
  The copies of its secret bytes are erased and its capabilities and
  pending requests dropped when it is closed. A party handed out before,
  to a handshake in progress, keeps its transport key until it is
  dropped (best effort, `CRYPTOGRAPHY.md` section 8).
- Tests: T-MI-2 and T-MI-3 (`tests/identities.rs`), the private network
  test (step 8: two identities of one node hold one peer differently),
  `tests/identities.rs` and `tests/supervisor.rs` (a deleted identity
  admits nobody, changes nothing, hands out no secret, and is taken
  down), T-MI-10, `identity::tests`, mutation faults CS46 to CS48 and
  CS60 to CS63.

### S41. An invitation capability admits requests only to the identity that issued it

- Mechanism: each local identity has its own active set (PROTOCOL.md
  section 12.3). A request is compared only with the set of the identity
  whose service it reached, and revoking changes only that set. There is
  no lookup across the sets of all identities.
- Tests: T-MI-4 (`tests/identities.rs`).

### S42. Stream isolation is per local identity and contact

- Mechanism: the backend makes an `IsolationGroup` from 16 CSPRNG bytes
  and keeps none. Each `LocalIdentity` keeps one group per remote
  identity (`LocalIdentity::isolation`), in memory only, and `link::dial`
  takes it from the identity, so two local identities never share a
  group, also for the same contact. A group is never derived from a key,
  an address, a name or a label (`TOR_INTEGRATION.md` section 3.2).
- Tests: `secret::tests` (groups are random and distinct),
  `tests/link.rs` (two local identities dial the same contact, each with
  its own group; T-MI-5), mutation fault CS22.

### S43. An inbound stream belongs to the identity whose service it reached

- Mechanism: each publication has its own control connection, listener
  and handle, and `serve` runs the accept loop of one service. The core
  runs that loop, under the supervisor of one identity, with that
  identity. No code looks up "the" local identity
  for a stream, and a stream is never tried against several identities.
  An Onion Service key belongs to one identity. Where a platform profile
  fixes the target port, this holds only while one identity receives
  streams (`TOR_INTEGRATION.md` section 4.3).
- Tests: `tests/system.rs` (two publications coexist, and a stream reaches
  only the service it was sent to), `tests/link.rs` (each local identity
  answers at its own service, and a card of one identity that names the
  service of another gets no session; T-MI-6), `tests/supervisor.rs`
  (the supervisors of two identities do not touch each other).

### S44. A peer of one local identity learns nothing about the others

- Mechanism: no message, field or card names a local identity other than
  the one of the session (S24), a card is signed by one identity and
  names only its own endpoints, and a peer is answered only with what
  that identity holds. Peers are never told the number of local
  identities, their labels or their state. What one identity sends does
  not depend on another's records; shared resources are the residual of
  `THREAT_MODEL.md` adversary S.
- Tests: T-MI-7 (`tests/identities.rs`: strangers at one identity change
  nothing at another).

### S45. Every outbound connection names the local identity it is made for

- Mechanism: `link::dial` takes the local identity the dial is for, and
  from it the party, the admission and the isolation group. There is no
  default identity and no global place one could come from. The dial
  scheduler, when it comes, queues each dial with its local identity.
- Tests: review of the signature of `link::dial`; T-MI-8
  (`tests/identities.rs`).

### S46. No local identity is process-wide state

- Mechanism: no static, global or thread-local value holds a key, a card,
  a party, contact state, a capability, a service or an isolation group;
  every function that needs one takes it as an argument or from the
  context of one identity. Process-wide are only the Tor backend and its
  status, the runtime, the configuration and the process-wide budget
  ceilings. An `Installation` is a value that holds its identities;
  `dev-chat` makes one identity per run as a test aid.
- Tests: `tests/structure.rs` in the core (no static item and no
  thread-local in the production code of any crate); review.

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
  logic allows it in the current state, and after a session was confirmed
  by the peer only once the local ContactAccept has gone out, because the
  peer takes application messages only after that. The application-side handler type
  that can only be constructed for a confirmed session comes with
  `monolith-core`.
- Tests: `message::tests`, T-PROTO-STATE (arbitrary event sequences in the
  property tests and in the `session_sequence` fuzz target), `tests::frames`
  (a chat message before confirmation, sent and received), fuzz target
  `handshake_responder` (nothing is delivered before confirmation, under
  the records none, blocked, requested and accepted).

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

- Mechanism: every queue capacity is a constant in `limits`.
  `clippy.toml` bans `tokio::sync::mpsc::unbounded_channel`. The network
  layer of Phase 3 holds no queue of its own: a link reads into one fixed
  buffer and reads again only when the session has taken it, and the
  accept loop's tasks are bounded by the handshake budget. The queue of
  pending contact requests of an identity is bounded in total and per
  capability (`MAX_PENDING_CONTACT_REQUESTS`,
  `MAX_PENDING_REQUESTS_PER_INVITATION`), and evicted strangers whose
  links have not ended are bounded by the budget for strangers.
- Tests: lint in CI, T-RES-3, the core tests of the accept loop (a flood
  beyond `MAX_INBOUND_HANDSHAKES` is closed, not queued), T-INV-5 and
  T-INV-6, `strangers::tests`, mutation faults CS20, CS26 and CS27.

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
  text configuration file whose lines are assembled from field values. The
  vault payload (`STORAGE.md` section 3.4) is binary, each value of it is
  decoded through the constructor of its type, and a payload that does
  not decode in full is refused as a whole; pending requests, the only
  peer-supplied text a stranger can send, are never stored.
- Tests: T-INJ-* (newline, delimiter, SQL, path and escape-sequence
  injection through every text field).

## Cryptography and secrets

### S17. Cryptographic randomness comes only from a CSPRNG

- Mechanism: the operating system source is read through `getrandom`,
  and only there: in `monolith-session` by `TransportSecretKey::generate`
  and by the random source that the resolver hands to the Noise library
  for the ephemeral keys of a handshake; in `monolith-tor` for the
  isolation tokens and the SAFECOOKIE client nonce (and, in the mock
  backend for tests, for mock keys); in `monolith-core` for the identity
  seeds and transport keys of new identities and of a rotation, for
  invitation capabilities, and for the jitter of the publication
  supervisor, which is not secret; in `monolith-storage` for the KDF
  salt, the vault key and the nonces of the vault; in `monolith-cli` for
  the identity seeds of the `dev-chat` test command and the message
  identifiers of the development commands. No other generator crate is a
  dependency of a product build. Non-cryptographic generator crates are banned by `deny.toml`. A
  failure of the source fails the operation; there is no fallback. A
  handshake with fixed ephemeral keys can be built only in the tests of
  that crate and in a build made with `--cfg fuzzing`; no cargo feature
  enables it, so it cannot be switched on through a dependency. The
  `monolith` binary refuses to compile with that configuration, through a
  `compile_error!` that no test exercises. `TransportSecretKey::from_bytes`
  refuses 32 zero bytes, which is what a buffer that the source never
  filled holds.
- Tests: `resolver::tests` (the source fills its buffer, two keys differ,
  a failing source leaves no key), `key::tests` (two generated transport
  keys differ, 32 zero bytes are not a key), `tests::handshake` (two
  handshakes differ in every message, a failing source fails the
  handshake), T-RNG-1, T-RNG-2.

### S18. Secrets and sensitive values do not appear in ordinary logs

- Mechanism: types that hold secrets, message content, onion addresses,
  identity keys, fingerprints, filenames, capabilities and isolation tokens
  do not derive `Debug` and do not implement `Display`. Their hand-written
  `Debug` prints `[redacted]`, or sizes and a type name where that is all
  there is to say. A type that is made only of such types may derive
  `Debug`, because the fields print themselves. A buffer of secret bytes
  (`Zeroizing<[u8; N]>`) is not such a type: it prints its bytes, so a
  type that holds one implements `Debug` by hand (the stored identity and
  rotation of `monolith_storage::record`). Error types carry no peer
  data. See `monolith_identity::redact`. The handshake and session types
  print `[redacted]`; a session prints its state, its frame and byte
  counters and whether it failed.
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
  ticket, so nothing of one session is input to another. A session drops
  its cipher keys when it ends, not when its object is dropped. The
  transport private key is the only secret that outlives a session; it is
  held in a type that erases it on drop and is passed to the Noise
  library by reference.
- Residual: erasure is best effort, and the Noise library does not erase
  what it holds itself (`CRYPTOGRAPHY.md` section 8).
- Tests: review of the public API of `monolith-session`; `Debug` tests of
  every type, among them what a session returns (`tests::frames`);
  `tests::frames` (the keys are gone after a violation, after a Close in
  either direction and after the stream closed).

### S35. A failure tells the peer nothing but that the stream closed

- Mechanism: the handshake functions return a message to send or an error,
  never both. `AuthenticatedSession::receive` returns an error and no
  frame. The protocol has no error message, and Close has an empty body
  and is not sent after a violation. What went wrong is a value for local
  use that carries no data from the peer; the caller closes the stream.
- Tests: T-ORACLE-4, `tests::handshake` and `tests::frames` (every failure
  returns an error and nothing to write), fuzz targets `handshake_responder`
  and `session_frames`.

### S37. A session ends at its limits, and nothing changes the limits

- Mechanism: `AuthenticatedSession` counts frames and ciphertext bytes in
  each direction and knows when it was established. `send` refuses every
  message but Close once the age, frame or byte limit is reached, and
  always leaves room for one Close. `receive` treats a frame beyond the
  frame or byte limit as a violation, and a frame that arrives once
  `SESSION_CLOSE_GRACE` has passed after the age limit as well. A file
  transfer extends the age limit only while it is active, and only if it
  was active before the limit was reached. The limits are constants of the protocol: a product build has
  no function that sets them. There is no rekey and no way to reset a
  counter; a new session is a new handshake.
- Tests: T-LIMIT (`tests::frames` with reduced limits, the property test
  that a session never sends more than its limits), fuzz target
  `session_frames` with the age limit of the protocol (from the limit on
  nothing but Close is sent, and a frame after the grace is a violation).

## Platform

### S25. Tails persistence is opt-in and explicit

- Mechanism: on Tails the default state mode is `Ephemeral`. A persistent
  vault is created only by a user command that names a location, and only
  inside the Persistent Storage mount. Monolith writes nothing to disk in
  ephemeral mode.
- Tests: T-PLAT-TAILS (filesystem diff after an ephemeral session).

### S31. Critical state changes are atomic

- Mechanism: the vault is replaced by write-to-temporary, fsync, rename,
  fsync-directory, and created under another name and linked into place.
  The identity key is never modified in place; every write produces a
  complete new file that carries it, with the whole installation in it.
  The contact store applies a change in memory under the lock of the
  contact and stamps it with a generation; whatever grants on the basis of
  the change (message 3, a returned link, the success of a user command)
  waits until that generation is in the vault, and a failed write fails
  the installation closed. A new invitation capability is listed, put
  into a card and admits requests only once its generation is durable,
  also when its creation was cancelled. A crash loses only changes
  nothing has used.
  The write is a job on the blocking pool that puts the vault back and
  publishes its outcome however the waiting task ends, so a cancelled
  wait loses neither the vault nor the lock of its directory, and the
  snapshot it writes is one cut through every contact store, never an
  operation half done. When the write fails, the job itself withdraws
  every session, whether or not a waiter is left.
- Tests: T-CRASH (`vault::tests`: a write or a creation stopped at every
  step leaves the old or the new vault, with every combination of what a
  crash keeps; `tests/store.rs`: every critical change stopped at every
  step of its write), `tests/store.rs`
  (`a_restart_reproduces_the_durable_state_exactly`), `persist::tests`
  (a cancelled wait, a failed write, many waiters on many threads),
  `contacts::tests` (snapshots taken while operations run at the bounds),
  `tests/credentials.rs`
  (`a_failed_write_withdraws_every_session_even_when_nobody_waits`),
  `tests/invitations.rs`
  (`an_invitation_is_handed_out_only_once_it_is_durable`),
  `requests::tests`, mutation faults CS13 to CS15, CS35, CS36, CS45,
  CS51, CS55, CS64 and CS65.

### S32. No hidden network traffic

- Mechanism: Monolith has no update check, telemetry, crash reporting or
  time synchronization. The only traffic it causes is peer sessions and the
  Tor control and SOCKS exchanges needed for them.
- Tests: T-NET-1 (idle run produces no traffic besides Tor's own).
