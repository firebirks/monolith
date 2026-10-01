# Monolith

Monolith is an experimental peer-to-peer messenger designed for direct
communication over Tor Onion Services. Two people who have exchanged contact
cards talk to each other directly, each running an Onion Service. There is
no server and no account.

Status: design phase. There is no working messenger in this repository yet.
The specifications are drafts under review, and the code is a workspace
skeleton. Nothing here has been audited. Do not rely on it for anything.

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
| `docs/CRYPTOGRAPHY.md` | Primitives, handshake, identity proof |
| `docs/TOR_INTEGRATION.md` | How Monolith uses Tor |
| `docs/TOR_CONTROL_SURFACE.md` | Every Tor control command used |
| `docs/PLATFORM_TAILS.md` | Tails |
| `docs/PLATFORM_WHONIX.md` | Whonix |
| `docs/STORAGE.md` | What is stored and how |
| `docs/RESOURCE_LIMITS.md` | Every limit and its reason |
| `docs/TEST_PLAN.md` | Tests, fuzzing, platform matrices |
| `docs/DESIGN_QUESTIONS.md` | Answers to the design questions, changes to the initial proposals, open questions |
| `docs/adr/` | Decision records |

## Building

Requires Rust 1.85.1 or later. `rust-toolchain.toml` pins the toolchain used
for development; building with another compiler at or above the minimum is
supported.

    cargo build --workspace --locked
    cargo test --workspace --locked

Checks run in CI:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo deny check
    cargo audit

## Layout

    crates/monolith-identity   identity and endpoint types
    crates/monolith-protocol   limits, messages, session states
    crates/monolith-tor        Tor backend interface
    crates/monolith-storage    storage policy types
    crates/monolith-core       application core
    crates/monolith-cli        the monolith binary
    crates/monolith-desktop    desktop front end (placeholder)
    integrations/tails         Tails control port profile (draft)
    integrations/whonix        Whonix profile and firewall drop-in (draft)
    fuzz/                      fuzz targets (planned)
    docs/                      specifications and decision records

## Security

See `SECURITY.md`. Early releases will not have had an independent audit and
will say so.

## License

MIT. See `LICENSE`.
