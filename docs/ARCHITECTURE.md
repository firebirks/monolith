# Architecture

Status: Phase 3 complete (tag `phase-3-complete`); Phase 4 not started.
The protocol core, the session layer and the integration with a Tor the
system runs are implemented and verified; the contact store, storage and
the interface are still design. `STATUS.md` records what is implemented
and verified. This document describes the structure the phases build.

## 1. Shape

Monolith is one process. A front end (CLI or desktop) talks to a core. The
core talks to peers through a Tor backend and to the disk through a storage
layer.

    monolith-desktop   monolith-cli
            \             /
             v           v
            monolith-core
          /    |      |     \
         v     |      v      v
     session   |   storage   tor (TorBackend trait)
         |     |      |        |
         v     v      |        +-- SystemTorBackend
        protocol      |        +-- MockTorBackend
            |         |        +-- (future) ArtiBackend
            v         |
         identity <---+

Dependencies point downwards only. Since Phase 3 `monolith-core` puts the
streams of the Tor backend under the sessions of `monolith-session`
(`link`) and holds the connection budgets (`budget`).

| Crate | Owns | Must not |
| --- | --- | --- |
| `monolith-identity` | identity, transport and endpoint public key types, fingerprints, signing and verification, redaction wrappers | know about sessions, Tor or files; draw randomness |
| `monolith-protocol` | limits, frame and message encoding, contact cards, the credentials of a contact, the session state machine | do cryptography other than verifying a card; open sockets, touch files, know which Tor is used |
| `monolith-session` | the handshake, the encrypted session, the transport secret key, every call to the Noise library, the random source | open sockets, touch files, read a clock, know about contacts beyond the record it is handed |
| `monolith-tor` | the `TorBackend` trait and its implementations: SOCKS5 client, control client, listener | know about identities' meaning, messages or contacts |
| `monolith-storage` | the vault, the message store, transfer files | change contact or session state on its own, interpret peer input |
| `monolith-core` | contacts, sessions, queues, policy, budgets, the command and event interface | contain UI code, socket code or file formats |
| `monolith-cli` | argument parsing, terminal output | contain protocol logic |
| `monolith-desktop` | windows and widgets | contain protocol logic |

Three boundaries are enforced by dependencies, not by convention:

- Only `monolith-tor` opens sockets. The lint in `clippy.toml` bans the
  socket constructors and UDP everywhere else, because the standard
  library and tokio would allow them in any crate. Protocol and session
  code is sans-IO: it consumes and produces byte buffers, and takes the
  time as an argument, which also makes it directly fuzzable.
- Only `monolith-session` links the Noise library and the crates behind
  it. Contact logic, storage and front ends see plaintext messages on one
  side and opaque bytes for the stream on the other.
- Only `monolith-storage` writes files. Reads outside it are limited to a
  fixed list: configuration files, `/etc/os-release` and the Whonix marker
  files for platform detection, Tor's cookie file where cookie
  authentication is used, and the swap state for `monolith doctor`.
- Only front ends write to the terminal or screen.

Tor transport does not define identity; the GUI does not define protocol
state; storage does not mutate protocol state.

### 1.1 Local identities

One installation may hold several local identities, and one process may
run several of them at the same time. A local identity is a Monolith
identity of its own, with its own identity key, transport key and Onion
Service key (S39). It is not a profile name, an account of the interface
or an alias, and two local identities are as unrelated on the wire as two
users on two machines.

    process
      +-- identity A   keys A, onion service A, contacts A, capabilities A
      +-- identity B   keys B, onion service B, contacts B, capabilities B
      +-- shared       Tor backend and its status, runtime, supervisor,
                       storage task, configuration, process-wide budgets

Everything that depends on an identity belongs to the context of one
local identity: its keys and `LocalParty`, its cards and active set of
invitation capabilities, its contacts, blocked, declined and former
contacts, verification marks and aliases, pending requests in both
directions, queued messages and history, isolation groups, its published
service with the accept loop of that service, and its budgets. State is
keyed by the local identity, and for a peer by the local and the remote
identity; nothing identity-dependent is process-wide (S40 to S46).

- Inbound. Each local identity publishes its own Onion Service. The accept
  loop of a service runs the responder with the party and the admission
  function of the identity that owns the service, so the service a stream
  arrived at says which identity it addresses. No code asks for "the" local
  identity, and a stream is never tried against several identities.
- Outbound. Every dial names the local identity it is made for: its party,
  its admission function, its isolation group for that peer. There is no
  default identity to fall back to.
- Concurrency. Identities are independent contexts in one process; several
  can be online at once. Switching identity in an interface changes what
  is shown, not which identities run.
- Labels. A local label such as "Personal" or "Project" is local metadata.
  It never enters a card, a handshake, Tor or any other peer-visible data,
  and it is not a credential or a unique name.
- Not multi-device. Several identities in one installation are unrelated
  identities. One identity on several devices (a root identity with device
  keys) is a different, later design and is not modelled by this.

The peer does not learn how many identities a process holds or which
others exist (S44); the protocol is unchanged by it (PROTOCOL.md section
1). What can still link identities that run together is in
`THREAT_MODEL.md` adversary S.

State in Phase 3: no code holds a local identity as global state. `link`
takes the party, an admission function and the isolation group as
arguments, `serve` runs one service, the Tor backend holds several
publications at once and keeps no identity state, and `Budgets` is a
value, not a singleton. `dev-chat` makes one identity per run as a test
aid. The contact store and the vault of Phase 4 are built with
identity-scoped records from the start, even if the first interface
offers one identity; which phase offers several in the interface is not
decided.

### 1.2 Admission and credentials

Which transport key stands for a contact is held in its `Credentials`
(`monolith-protocol`, PROTOCOL.md section 11.4): the active card, an
authorized and a pending successor, the retired key. Authentication says
who the peer is; the credentials say whether that peer is the contact on
this session.

- Admission is one step. `link::dial` and `link::answer` call the
  admission function when the peer is authenticated, after the Tor stream
  and the handshake messages that authenticate it. The function looks up
  the record of the identity as it is then, admits the authenticated peer
  with the record borrowed mutably, so that the standing and the change
  of the credentials are one call, and keeps the `Withdrawal` of the link
  with the session. Nothing between the lookup and the end of the step
  waits.
- A dial calls the admission function before it writes message 3, and
  writes it only if the peer may learn the local identity
  (`Admission::may_learn_local_identity`, read from the standing of the
  session the function returned), racing the write against the
  withdrawal; so an outbound session is always a contact's. The slot for
  strangers of an inbound session also follows the standing of the
  session.
- A link ends at the deadlines of its session and on every failure, by
  one path that also gives back its slot for strangers; a link that is
  over refuses every later call (`RESOURCE_LIMITS.md` section 4). A
  message that ends the session, the first one of a stranger or a Close,
  ends the link before it is returned.
- Retirement withdraws. When a successor is promoted, by an admission or
  by the user's confirmation, every session that was admitted as a
  contact's and whose card no longer states the active key
  (`Credentials::authorizes`) is withdrawn before anything else is done
  with the contact, the duplicate rule included. The store keeps the
  withdrawal of contact sessions only: a session admitted with another
  standing is left on the path of a stranger, because ending it early
  would show the peer that it is held as a contact. Deleting or blocking
  the record of an identity withdraws its sessions in the same way, so a
  dial whose contact is deleted or blocked after the admission writes no
  message 3, or stops writing it.
- Applying is checked again. A message a link has returned was decided
  when it was taken; what it would change in the contact state, a
  `MarkAccepted`, a request, an EndpointUpdate, is applied under the
  lock of that state and only while the session still stands for the
  contact (`Credentials::authorizes` for its card, or the link not
  withdrawn). `Credentials::announce` makes this check itself.
- The admission function runs inside the handshake deadline and must not
  block or wait: it takes the lock, works on memory and lets go. No lock
  of the contact state is held across the `await` of `dial`.
- Phase 3 holds no contact state. It provides the credential type, the
  admission entry points, the withdrawal and the duplicate rule with
  credential standing, and tests them with a minimal store.

What the contact store of Phase 4 has to add: one lock, or one
transaction, per contact around lookup, admission and keeping the
withdrawal; withdrawal of the sessions of a retired key inside the same
step as the promotion, and of every session of an identity whose record
is deleted or blocked inside that step; the confirmation and import actions of the
interface on top of `Credentials::confirm`, `import` and `replace`;
persisting the credentials atomically with the rest of the contact record
(S31), either inside the step or before its result is used, so that a
crash cannot bring a retired key back; forgetting the withdrawals of links
that ended (`Withdrawal::is_ended`), including those `answer` refused for
lack of a slot after the admission had run; the timing of a rotation and
of the switch to the new key; the dial card the user confirmed, kept
apart from the credentials; and one evaluation of a card for the import
by the user and for the admission of a peer. Today
`Credentials::import` holds a card of another key that is older than an
announced successor as pending, while admission and announcement call it
stale. Both give no standing, neither lowers the epoch of the active
card, and the pending card takes over only if the user confirms exactly
that card; the store has to bring the two under one rule.

## 2. Processing order for peer input

Every byte from a peer passes these stages in order. A failure at any stage
closes the session.

    bounded        frame length checked against the limit for the state
    counted        the session's age, frame and byte limits
    decrypted      authenticated decryption of the frame
    framed         header, padding and body length checked
    gated          message type legal in the current session state
    parsed         the body, by type
    validated      field rules: ranges, UTF-8, text rules, signatures
    authenticated  the identity the handshake established is who may send this
    authorized     local policy: contact state, block list, budgets, rates
    applied        state change or event to the front end

Persistent state is written only at "applied".

## 3. Runtime

- One Tokio runtime. The core's tasks:
  - a supervisor that owns all other tasks;
  - one task per session (handshake, then frame loop);
  - one accept loop per published service, which owns the handshake tasks
    it starts in a `JoinSet`, at most `MAX_INBOUND_HANDSHAKES` of them, and
    aborts them when it ends;
  - a dial scheduler with at most `MAX_CONCURRENT_DIALS` dials;
  - Tor control connections: one held by each published service, which
    ends with it, and short ones for status queries;
  - one storage task; vault writes and KDF work run on the blocking pool;
  - one writer task per active file transfer.
- Every channel is bounded, with its capacity in `limits.rs` (S12).
  Unbounded channel constructors are banned by lint.
- Every task is owned by the supervisor through a join handle and has a
  cancellation signal. Nothing is detached. Shutdown cancels, waits with a
  deadline, then drops.
- No task per message or per frame.
- Backpressure is end to end: when the front end's event queue or a disk
  writer is full, the session task stops reading from its stream.

Panic policy: a panic is a bug. Lints forbid the usual sources in non-test
code, and release builds check integer overflow. If a session task panics
anyway, the supervisor observes it through the join handle, counts it,
closes that session and continues. Panics unwind so that this works and so
that secret types are dropped and zeroized. A panic in the supervisor, the
storage task or the control task ends the process cleanly.

## 4. Core interface

Front ends send commands and receive events. Both are plain typed values.
Events come through one bounded queue (`MAX_UI_EVENTS`) with coalescing for
state-like events (`RESOURCE_LIMITS.md` section 8). A command or event
that concerns an identity names the local identity it belongs to (section
1.1); only those about Tor and the process do not.

Commands (version 1): create or unlock identity; lock; show own contact
card; create a card with a new invitation capability and a local label;
revoke an invitation capability; set request mode; add contact card;
accept, decline, block a request; block, unblock, delete a contact; mark
verified; send message; offer file; accept, reject, abort a transfer;
confirm endpoint update; set profile; shut down. For several local
identities, later: create another identity, set its local label, take it
online or offline, delete it (`STORAGE.md` section 1.1).

Events (version 1): Tor state; service state; peer state per contact;
identity mismatch; request count changed; message received; message
acknowledged; file offered; transfer progress and result; endpoint update
pending; queue full notices.

Front ends must be able to show these states distinctly, and none of them
may be shown as "secure" merely because Tor is connected:

- Tor: disconnected, connecting, service publishing, service available.
- Peer: offline, connecting, handshaking, authenticated but not confirmed,
  online contact, identity mismatch.
- Contact trust: pinned, or verified out of band.
- Identity: ephemeral or persistent.

These are the enums in `monolith-core` today.

Contact cards and invitations (PROTOCOL.md 12.2, 12.3). Terms for the
front ends of later phases: "Generate contact card", "Generate another
card", "Revoke card access", and the user's own note of how a card was
shared, "Publicly shared" or "Privately shared". That note and the label
of a capability ("Website", "Directory A", "Private QR for Bob") are local
data and never enter the signed card or the wire. The interface must not
suggest that Monolith knows whether an exported card is still private,
that taking a card off a directory or a website withdraws it, or that
revoking a card's access affects contacts already accepted. A card can be
issued without a capability; the user chooses that, and Monolith does not
add one.

Local identities (section 1.1). A front end of a later phase may show a
current identity with a list to switch to ("Personal", "Project",
"Pseudonym"), or the notifications of several active identities at once,
each marked with the identity it belongs to. The labels are the user's
convenience: they are not unique, not usernames and not shown to peers.
Security-relevant views still show the fingerprint of the local identity
and of the peer. Switching the identity that is shown takes no identity
offline.

## 5. Command-line interface

    monolith status
    monolith identity show
    monolith identity fingerprint
    monolith contact-card show
    monolith contact list
    monolith contact add <card>
    monolith tor status
    monolith doctor
    monolith version

No command prints a private key. There is no export of secrets in version
1. Peer-supplied text is escaped before it is written to a terminal.

The identity, card and contact commands act on one local identity. Once
an installation can hold several, they take the identity to act on as an
argument; none of them picks one silently.

Phase 3 implements `tor status` and `doctor`, with the options `--socks`,
`--control` and `--control-auth`, and the development command
`dev-chat serve` and `dev-chat dial` for the two-node test on a private Tor
network (identities and services in memory only). The other commands
report that they are not implemented yet.

`monolith doctor` checks and reports, without revealing secrets or
addresses: SOCKS reachability; control access and which authentication the
endpoint offers; whether an Onion Service can be published; Tor version
against the recommended minimum; platform detected; storage mode and data
directory permissions; whether swap is in use.

The CLI exists before the GUI because integration tests drive it.

## 6. Logging and errors

- `tracing`, with events limited to component state, error category,
  bounded counters and protocol version.
- Sensitive values cannot be logged by accident: their types print
  `[redacted]` and error types carry no peer data (S18). A CI test runs the
  hostile-peer suite at maximum verbosity and searches the output for every
  secret and every peer string it used.
- Malformed input is counted, not logged per event. Counters are bounded
  and a flood cannot grow the log.
- Each library crate has its own error enum (`ProtocolError`, `TorError`,
  `StorageError`). `anyhow`-style dynamic errors are allowed only in the
  binaries' top level.
- A peer is never told why its input was rejected.

## 7. Configuration

Read at start from, in order: built-in defaults, platform defaults chosen by
detection, `/etc/monolith.d/*.conf`, `/usr/local/etc/monolith.d/*.conf`, the
user's configuration file, command-line options. Later sources override
earlier ones. The drop-in directories follow what Whonix asks of
applications.

Configuration holds endpoints, modes and limits that may be lowered. It
never holds anything a peer supplied (S28), and no setting can disable an
invariant: there is no "allow direct connections" or "skip verification"
switch.

## 8. What is not sent to peers

No operating system, distribution, Tails or Whonix indication, architecture,
locale, hostname, username, toolkit, build hash or version string. Nothing
about other local identities: not their number, keys, cards, endpoints,
labels or state. Nothing
is negotiated: the protocol version is a label that both sides hash into
the handshake and that never appears on the wire. Timestamps are not
transmitted.

## 9. Security-sensitive changes

Changes to these areas are security-sensitive: protocol parsing, identity,
cryptography, randomness, Tor control, SOCKS, storage encryption,
persistence migration, file transfer, path handling, contact acceptance,
endpoint migration, and anything in `limits.rs`.

For such a change:

- tests come with it;
- the specification is updated in the same change if behavior changes;
  the normative documents define intended behavior, `STATUS.md` records
  what is implemented, and a disagreement between code and a normative
  document is a bug to resolve, never a silent override in either
  direction;
- affected invariants are named in the description;
- no unrelated refactoring rides along;
- a second person reviews it.

## 10. Toolchain and dependencies

- Edition 2024.
- Two compiler versions are kept apart:
  - The minimum supported Rust version (MSRV) is 1.85.1, the rustc in
    Debian 13. It is declared as `rust-version` in the workspace and is what
    anyone, including a distribution, needs to build Monolith.
  - The developer toolchain is pinned in `rust-toolchain.toml` and may be
    newer. It fixes the versions of rustfmt and clippy so that formatting
    and lints give the same result for everyone. Nobody is required to
    build with it.
- CI builds and tests with the MSRV, with the developer toolchain and with
  the current stable compiler.
- A dependency must not raise the MSRV unnoticed. The dependency resolver
  takes `rust-version` into account, and the MSRV job builds with the
  locked versions, so an update that needs a newer compiler fails there.
- Raising the MSRV is a change of its own: one commit that updates
  `rust-version`, the CI variable and this section, with the reason, and an
  entry in the release notes of the next release.
- `Cargo.lock` is committed.
- `unsafe` is forbidden in every Monolith crate by a workspace lint. If it
  ever becomes necessary it gets its own small crate, an ADR and tests.
- Dependencies are chosen for maintenance, audit history, advisories,
  transitive size, unsafe surface and licence; the reasoning is recorded in
  the ADR that introduces them. `cargo deny` enforces advisories, licences,
  sources (crates.io only) and bans.
- Nothing is published to crates.io without explicit authorization
  (`publish = false` on every crate).
- The project is MIT licensed. Every crate carries `license = "MIT"`
  through the workspace metadata.

## 11. Releases

Planned, not yet in place:

- reproducible builds where practical: pinned toolchain, locked
  dependencies, path remapping (`--remap-path-prefix` with
  `--remap-path-scope`);
- dependency information embedded in binaries (`cargo auditable`) and an
  SBOM (CycloneDX);
- SHA-256 checksums and a signed release manifest. minisign is the
  proposed signing tool: one static public key and fully offline
  verification, which suits users on Tails and Whonix. Sigstore's keyless
  mode ties verification to online infrastructure and an OIDC identity;
- signed source tags.

No update check, no telemetry, no crash reporting. Updates arrive through
the platform's packaging.

## 12. Phases

| Phase | Content |
| --- | --- |
| 0 | Design, specifications, workspace skeleton. |
| 1 | Protocol core without cryptography or Tor: framing, encoding, identity types, contact cards, state machine; unit, property and fuzz tests. |
| 2 | Cryptographic session: Noise XK handshake, the contact card as certificate of the transport key, encrypted frames, session limits, test vectors, hostile-handshake tests. |
| 3 | System Tor: SOCKS5, control client, mock backend, two-node CLI chat. |
| 4 | Contacts and queue: acceptance, pinning, vault, duplicate resolution, reconnect scheduling. |
| 5 | File transfer. |
| 6 | Tails. |
| 7 | Whonix. |
| 8 | GUI. |
| 9 | Hardening: attack simulation, fuzzing corpus, benchmarks, dependency audit. |
| 10 | Audit candidate: protocol freeze. |

A phase starts after the previous one has been reviewed.

Preconditions beyond review:

- Phase 2 needs the session layer decision of ADR 0002.
- Phase 6, and any claim that Monolith works on Tails, needs the Tails
  checks of `PLATFORM_TAILS.md` section 3.4 to have been made on a current
  Tails release. They do not gate the generic system Tor backend of Phase
  3 (`DESIGN_QUESTIONS.md` T3-1).
- Phase 7 needs the Whonix isolation measures of `PLATFORM_WHONIX.md`
  section 4.5 to have been tested with two Workstations on one Gateway.

## 13. Open items

A2. Whether to keep the minimum Rust version at Debian stable's. Current
    dependency candidates build with 1.85, but the ecosystem moves faster
    than Debian.

A3. Configuration file format: a minimal `key = value` parser in-house, or
    the `toml` crate.

A4. Command-line parser: hand-written as now, `lexopt`, or `clap`.
