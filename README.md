# Monolith

[![CI](https://github.com/firebirks/monolith/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/firebirks/monolith/actions/workflows/ci.yml?query=branch%3Amain)

Monolith is an experimental peer-to-peer messenger for direct communication
over Tor Onion Services. Two people who have exchanged contact cards talk
to each other directly, each running an Onion Service. There is no server
and no account: no third party stores or forwards messages, apart from the
Tor network that carries the connection.

> **Experimental software.** There is no release, and no independent
> security audit has been done. Do not rely on Monolith for high-stakes or
> production use yet.

## What it is

- One-to-one messaging and file transfer between accepted contacts.
- Onion Service v3 only, using a Tor that already runs on the system.
  Monolith does not bundle, install or start Tor. On Tails and Whonix that
  means the platform's Tor, through the platform's control port filter.
- A contact is identified by a public key, verified by comparing a
  fingerprint. The onion address is where the contact is reached, not who
  the contact is.
- An authenticated, encrypted session between the two applications, in
  addition to what Tor provides.
- No telemetry, analytics, crash reports or update checks: the client's
  only connections are to the configured Tor.
- A profile kept in memory only ("Start without saving"). Monolith writes
  none of its identities, contacts, messages or settings to disk, and they
  are lost when it exits; files you save and contact cards you export are
  written where you choose. This does not stop the operating system or Tor
  from writing to disk (swap or the page file, hibernation, crash dumps,
  Tor's own state).

## What it is not

- It is not a replacement for Tor and adds nothing to Tor's anonymity.
- It does not defend against an adversary who can watch both ends of a
  connection, or against a compromised computer.
- It cannot make a received file safe to open. A completed transfer proves
  the file is the one its sender declared, not that its content is safe;
  Monolith never opens a received file.
- It does not hide whether you are online. Anyone who holds your contact
  card can try to connect and so see whether your endpoint is reachable.
- It has no offline delivery. A message to a contact who is not running
  Monolith stays queued on your side and is sent when both of you are
  running it at the same time.
- It has no groups, calls, read receipts, typing indicators or link
  previews.
- It is not compatible with TorChat or any other messenger.

The full list of limits is in
[`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md#6-limits-and-non-goals),
section 6.

## Status

Early development, with no release. The protocol core (encoding, contact
cards, session state logic) and the session layer (the handshake and the
encrypted frames) are libraries that do no I/O. Each later phase ended
with a final verification, recorded in
[`docs/DESIGN_QUESTIONS.md`](docs/DESIGN_QUESTIONS.md), and a tag.

| Phase | What it added | Tag | Verification |
| --- | --- | --- | --- |
| 3 | Integration with the Tor the system runs: a SOCKS5 client, a control client for status and for publishing an Onion Service, sessions over Tor streams with their budgets and deadlines, and a two-node test over a private Tor network | `phase-3-complete` | [10.4](docs/DESIGN_QUESTIONS.md#104-final-verification-and-completion) |
| 4 | The contact store of each local identity, the encrypted vault that keeps it, invitations, several local identities, the publication supervisor and the rotation of a transport key, with a development node in the command-line binary | `phase-4-complete` | [11.6](docs/DESIGN_QUESTIONS.md#116-final-verification-and-completion) |
| 5 | The messaging runtime, without a user interface: an encrypted message store, delivery acknowledged only once the receiver stored a message, the session manager, reconnecting, and commands and events for an interface | `phase-5-complete` | [12.8](docs/DESIGN_QUESTIONS.md#128-final-verification-and-completion) |
| 6 | The desktop client for Linux and Windows ([`docs/DESKTOP.md`](docs/DESKTOP.md)): one window over that runtime, with a design system of its own | `phase-6-complete` | [13.3](docs/DESIGN_QUESTIONS.md#133-final-verification-and-completion) |
| 7 | Hardening, with no new feature: the dial plan during a rotation of a contact's key, a sender's words kept apart from the client's, the checks of local files on Windows, file messages refused as the protocol says, stores that still open after a limit is lowered, the liveness of sessions, and the rates of each contact | `phase-7-complete` | [14.3](docs/DESIGN_QUESTIONS.md#143-final-verification-and-completion) |
| 8 | File transfer between accepted contacts ([`docs/adr/0013-file-transfer.md`](docs/adr/0013-file-transfer.md)): offered and explicitly accepted, sent in chunks over the session, resumed across lost sessions and restarts, staged encrypted, verified before it counts as received, and saved as a new file where the user chooses | `phase-8-complete` | [15.5](docs/DESIGN_QUESTIONS.md#155-final-verification-and-completion) |

## Platforms

- Built for: Linux (X11 or Wayland) and Windows 10 and 11, on x86_64.
  macOS is not built or tested.
- Verified so far: the tests run in CI on Ubuntu; the final verification
  of Phase 8 used Windows 11 and Debian 12 and 13 (a container and WSL).
  Nothing has run yet on a clean machine, on Windows 10, or on a real
  Tails or Whonix system.
- Releases: none. The platforms of a first test release are not decided.
- Tails and Whonix: draft profiles in [`integrations/`](integrations),
  experimental and unverified.

What the client needs at run time
([`packaging/README.md`](packaging/README.md#what-the-binary-needs-at-run-time)):

- Linux: X11 or Wayland, OpenGL 2.1 or OpenGL ES 2.0 (Mesa), libxkbcommon,
  and a glibc at least as new as that of the machine that built it.
- Windows: Windows 10 or 11, 64-bit, with an OpenGL 2.1 driver.
- A running Tor: the Debian `tor` service or Tor Browser on Linux, Tor
  Browser or the Tor Expert Bundle on Windows. How to point Monolith at
  each is in [`packaging/linux/README.txt`](packaging/linux/README.txt) and
  [`packaging/windows/README.txt`](packaging/windows/README.txt).

No binaries are published. A Linux binary built on Debian 13, as the
archives made so far were, needs glibc 2.39 or newer: it does not start on
Debian 12 or Ubuntu 22.04. Building from source on the system that will run
it avoids this.

## Building

Requires Rust 1.85.1 or later (`rust-version` in `Cargo.toml`).
`rust-toolchain.toml` pins the toolchain used for development (1.95.0);
building with another compiler at or above the minimum is supported.

    cargo build --workspace --locked
    cargo test --workspace --locked

The desktop client:

    cargo run --release -p monolith-desktop

Release builds and the Linux and Windows archives:
[`packaging/README.md`](packaging/README.md).

## Checks

CI ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)) runs on every
push, on Linux only:

| Job | What it runs |
| --- | --- |
| fmt | `cargo fmt --all --check` |
| clippy | `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| test | `cargo test --workspace --locked` with the pinned toolchain |
| doc | `cargo doc --workspace --no-deps --locked`, warnings denied |
| msrv | `cargo check --all-targets` and `cargo test` with Rust 1.85.1 |
| stable | `cargo test` with the newest stable compiler |
| fuzz-build | the fuzz targets compile with nightly (no fuzzing) |
| deny | `cargo deny check`: advisories, bans, licenses, sources |
| audit | `cargo audit` |

Longer checks run outside CI, in the final verification of a phase: the
Windows build and tests, the fail-closed network test
([`tests/network`](tests/network/README.md)), the test over a private Tor
network ([`tests/tor-network`](tests/tor-network/README.md)), fuzzing, and
the mutation lists of [`mutation/`](mutation/README.md).

## Documentation

| Document | Content |
| --- | --- |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Components, runtime, phases |
| [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md) | Adversaries, defenses, limits |
| [`docs/SECURITY_INVARIANTS.md`](docs/SECURITY_INVARIANTS.md) | Properties that must always hold |
| [`docs/PROTOCOL.md`](docs/PROTOCOL.md) | Wire protocol |
| [`docs/CRYPTOGRAPHY.md`](docs/CRYPTOGRAPHY.md) | Primitives, handshake, how keys are bound to identities |
| [`docs/TOR_INTEGRATION.md`](docs/TOR_INTEGRATION.md) | How Monolith uses Tor |
| [`docs/TOR_CONTROL_SURFACE.md`](docs/TOR_CONTROL_SURFACE.md) | Every Tor control command used |
| [`docs/PLATFORM_TAILS.md`](docs/PLATFORM_TAILS.md) | Tails |
| [`docs/PLATFORM_WHONIX.md`](docs/PLATFORM_WHONIX.md) | Whonix |
| [`docs/STORAGE.md`](docs/STORAGE.md) | What is stored and how |
| [`docs/RESOURCE_LIMITS.md`](docs/RESOURCE_LIMITS.md) | Every limit and its reason |
| [`docs/TEST_PLAN.md`](docs/TEST_PLAN.md) | Tests, fuzzing, platform matrices |
| [`docs/DESIGN_QUESTIONS.md`](docs/DESIGN_QUESTIONS.md) | Answers to the design questions, changes to the initial proposals, open questions |
| [`docs/DEPENDENCIES.md`](docs/DEPENDENCIES.md) | Third-party crates in use and what was checked |
| [`docs/DESKTOP.md`](docs/DESKTOP.md) | The desktop client: boundary, screens, keyboard, accessibility, limits |
| [`packaging/README.md`](packaging/README.md) | Building and packaging the desktop client |
| [`docs/adr/`](docs/adr) | Decision records |

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
    mutation/                  single-fault lists and their runner
    verification/              outputs of the Phase 4 final verification
    docs/                      specifications and decision records

## Security

The security policy is in [`SECURITY.md`](SECURITY.md). A private channel
for reporting vulnerabilities is not set up yet; until it is, please do not
put the details of a vulnerability in a public issue. No independent
security audit has been done, and early releases will say so.

## License

MIT. See [`LICENSE`](LICENSE).
