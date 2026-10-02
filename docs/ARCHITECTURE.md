# Architecture

Status: Phase 2. The protocol core and the session layer are implemented;
everything that touches the network or the disk is still design. This
document describes the structure the phases build.

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

Dependencies point downwards only. `monolith-core` does not use
`monolith-session` yet; that comes with Phase 3.

| Crate | Owns | Must not |
| --- | --- | --- |
| `monolith-identity` | identity, transport and endpoint public key types, fingerprints, signing and verification, redaction wrappers | know about sessions, Tor or files; draw randomness |
| `monolith-protocol` | limits, frame and message encoding, contact cards, the session state machine | do cryptography other than verifying a card; open sockets, touch files, know which Tor is used |
| `monolith-session` | the handshake, the encrypted session, the transport secret key, every call to the Noise library, the random source | open sockets, touch files, read a clock, know about contacts beyond the record it is handed |
| `monolith-tor` | the `TorBackend` trait and its implementations: SOCKS5 client, control client, listener | know about identities' meaning, messages or contacts |
| `monolith-storage` | the vault, the message store, transfer files | change contact or session state on its own, interpret peer input |
| `monolith-core` | contacts, sessions, queues, policy, budgets, the command and event interface | contain UI code, socket code or file formats |
| `monolith-cli` | argument parsing, terminal output | contain protocol logic |
| `monolith-desktop` | windows and widgets | contain protocol logic |

Three boundaries are enforced by dependencies, not by convention:

- Only `monolith-tor` links a networking crate. Protocol and session code
  is sans-IO: it consumes and produces byte buffers, and takes the time as
  an argument, which also makes it directly fuzzable.
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
  - one accept loop per published service;
  - a dial scheduler with at most `MAX_CONCURRENT_DIALS` dials;
  - one Tor control task;
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
state-like events (`RESOURCE_LIMITS.md` section 8).

Commands (version 1): create or unlock identity; lock; show own contact
card; create and revoke invitation; set request mode; add contact card;
accept, decline, block a request; block, unblock, delete a contact; mark
verified; send message; offer file; accept, reject, abort a transfer;
confirm endpoint update; set profile; shut down.

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
| 0 | Design, specifications, workspace skeleton. This phase. |
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
