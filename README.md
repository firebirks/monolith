# Monolith

Monolith is an experimental peer-to-peer messenger designed for direct
communication over Tor Onion Services. Two people who have exchanged contact
cards talk to each other directly, each running an Onion Service. There is
no server and no account.

Status: early development. The protocol core (encoding, contact cards,
session state logic) and the session layer (the handshake and the
encrypted frames) are implemented as libraries that do no I/O. The
integration with a Tor the system runs is implemented and was verified at
the end of Phase 3 (tag `phase-3-complete`), including a two-node test
over a private Tor network: a SOCKS5 client, a control client for status
and for publishing an Onion Service, sessions over Tor streams with their
budgets and deadlines. Phase 4, complete and verified (tag
`phase-4-complete`), adds the contact store of each local identity, the
encrypted vault that keeps it, invitations, several local identities, the
publication supervisor and the rotation of a transport key, with a
development node in the command-line binary. Phase 5, complete and
verified (tag `phase-5-complete`), adds the messaging runtime without a
user interface: an encrypted message store, delivery acknowledged only
once the receiver stored a message, the session manager, reconnecting, and
commands and events for a later interface. Phase 6, complete and verified
(tag `phase-6-complete`), adds the desktop client for Linux and Windows
(`docs/DESKTOP.md`): one window over that runtime, with a design system of
its own. Phase 7, complete and verified (tag `phase-7-complete`),
hardens what Phases 5 and 6 built, with no new feature: the dial plan
during a rotation of a contact's key, a sender's words kept apart from
the client's, the checks of local files on Windows, file messages
refused as the protocol says, stores that still open after a limit is
lowered, the liveness of sessions, and the rates of each contact. File
transfer is not implemented. The Tails and Whonix integrations (control
port filters, the firewall rule) are drafts. `docs/DESIGN_QUESTIONS.md`
sections 10.4, 11.6, 12.8, 13.3 and 14.3 record the final verifications
of Phases 3 to 7. Nothing here has been audited. Monolith is still experimental software. Do not rely on it for
high-stakes or production use yet.

## What it is

- One-to-one messaging and file transfer between accepted contacts.
- Onion Service v3 only, using the Tor already on the system. On Tails and
  Whonix that means the platform's Tor, through the platform's control port
  filter.
- A contact is identified by a public key, verified by comparing a
  fingerprint. The onion address is where the contact is reached, not who
  the contact is.
- An authenticated, encrypted session between the two applications, in
  addition to what Tor provides.
- Able to run without leaving anything on disk.

## What it is not

- It is not a replacement for Tor and adds nothing to Tor's anonymity.
- It does not defend against an adversary who can watch both ends of a
  connection, or against a compromised computer.
- It cannot make a received file safe to open.
- It does not hide whether you are online. Anyone who holds your contact
  card can try to connect and so see whether your endpoint is reachable.
- It has no groups, calls, read receipts, typing indicators, link previews
  or offline delivery.
- It is not compatible with TorChat or any other messenger.

The full list of limits is in `docs/THREAT_MODEL.md`, section 6.

## Documentation

| Document | Content |
| --- | --- |
| `docs/ARCHITECTURE.md` | Components, runtime, phases |
| `docs/THREAT_MODEL.md` | Adversaries, defenses, limits |
| `docs/SECURITY_INVARIANTS.md` | Properties that must always hold |
| `docs/PROTOCOL.md` | Wire protocol |
| `docs/CRYPTOGRAPHY.md` | Primitives, handshake, how keys are bound to identities |
| `docs/TOR_INTEGRATION.md` | How Monolith uses Tor |
| `docs/TOR_CONTROL_SURFACE.md` | Every Tor control command used |
| `docs/PLATFORM_TAILS.md` | Tails |
| `docs/PLATFORM_WHONIX.md` | Whonix |
| `docs/STORAGE.md` | What is stored and how |
| `docs/RESOURCE_LIMITS.md` | Every limit and its reason |
| `docs/TEST_PLAN.md` | Tests, fuzzing, platform matrices |
| `docs/DESIGN_QUESTIONS.md` | Answers to the design questions, changes to the initial proposals, open questions |
| `docs/DEPENDENCIES.md` | Third-party crates in use and what was checked |
| `docs/DESKTOP.md` | The desktop client: boundary, screens, keyboard, accessibility, limits |
| `packaging/README.md` | Building and packaging the desktop client |
| `docs/adr/` | Decision records |

## Building

Requires Rust 1.85.1 or later. `rust-toolchain.toml` pins the toolchain used
for development; building with another compiler at or above the minimum is
supported.

    cargo build --workspace --locked
    cargo test --workspace --locked

The desktop client:

    cargo run --release -p monolith-desktop

Checks run in CI:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo deny check
    cargo audit

## Layout

    crates/monolith-identity   identity keys, signatures, fingerprints
    crates/monolith-protocol   limits, encoding, contact cards, messages,
                               frames, session logic
    crates/monolith-session    handshake and encrypted session
    crates/monolith-tor        Tor backend: SOCKS5 client, control client,
                               Onion Service publication, mock backend
    crates/monolith-storage    the vault and the message store: formats,
                               records, atomic replace
    crates/monolith-core       the contact store, local identities,
                               sessions over Tor streams, budgets, the
                               publication supervisor, messages, the
                               session manager and the runtime
    crates/monolith-runtime    what a front end may use of the runtime
    crates/monolith-cli        the monolith binary: Tor status, doctor,
                               development chat and development nodes
    crates/monolith-ui-model   the state of the desktop client (no egui)
    crates/monolith-ui-kit     its design system: theme, fonts, icons,
                               components (egui)
    crates/monolith-ui         its screens
    crates/monolith-ui-snapshot  a software renderer of egui, for tests
                               and pictures of the screens
    crates/monolith-desktop    the monolith-desktop binary: the window,
                               the bridge to the runtime, its files
    packaging/                 Linux and Windows archives of the client
    integrations/tails         Tails control port profile (draft)
    integrations/whonix        Whonix profile and firewall rule (draft)
    fuzz/                      fuzz targets for the protocol core, the
                               session layer, the Tor replies, the vault,
                               the contact store, the messages and the
                               state of the desktop client
    tests/network              fail-closed network test (Linux)
    tests/tor-network          two-node test over a private Tor network
    docs/                      specifications and decision records

## Security

See `SECURITY.md`. Early releases will not have had an independent audit and
will say so.

## License

MIT. See `LICENSE`.
