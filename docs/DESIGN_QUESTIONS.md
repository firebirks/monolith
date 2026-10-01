# Design questions

Phase 0 had to answer a fixed list of questions, check the initial
proposals against current sources, and leave nothing unresolved hidden in
code. This file is the index: the answers, the places where the design
departs from what was first proposed and why, and every open question with
the document that holds it.

Sources were read on 2026-10-01.

## 0. Status after the Phase 0 review

Phase 0 is accepted as the basis for implementing the protocol core. It is
not a frozen design.

Accepted for implementation in Phase 1: limits, error types, the binary
encoding, contact cards and endpoint sets, fingerprints, text validation,
the message and frame formats, the session state machine, contact
confirmation, and the duplicate-session rule. No network I/O, no Tor, no
session cryptography and no storage encryption are part of Phase 1.

Provisional, and not to be built on yet:

| Area | State | Decided where |
| --- | --- | --- |
| Session cryptography: Noise pattern, identity binding, or TLS | four options compared, none selected | ADR 0002 |
| Probing behavior of the handshake | depends on the above; the prologue binding is not access control | ADR 0002 |
| Tails Onion Service integration | blocked on experiments on a current Tails | PLATFORM_TAILS.md 3.4 |
| Whonix isolation and firewall integration | measures specified, untested | PLATFORM_WHONIX.md 4.5 |
| Vault KDF parameters | proposed, benchmark pending | STORAGE.md 3.3 |
| Padding block size | mechanism decided, value not | ADR 0003 |
| Message store | open | STORAGE.md 5 |
| GUI toolkit | open | ADR 0006 |

Decided in the review: the project is MIT licensed; the minimum supported
Rust version is 1.85.1 and is separate from the developer toolchain; an
identity has a set of endpoints, limited to one in version 1.

## 1. Answers

| # | Question | Answer | Where |
| --- | --- | --- | --- |
| 1 | Noise XX with an Ed25519 transcript proof, or TLS 1.3 | Not decided. Four constructions are compared in ADR 0002: Noise XX with a transcript proof, Noise NN with a transcript proof, a certified persistent static key, and TLS 1.3 with pinned raw public keys. The decision is due before Phase 2 and needs a cryptographer's review. | ADR 0002, CRYPTOGRAPHY.md |
| 2 | Canonical wire encoding | Fixed-layout binary, hand-written encoders and decoders, no serialization framework. One valid encoding per structure. | ADR 0003, PROTOCOL.md 2, 5 |
| 3 | Fingerprint encoding | SHA-256 over a prefix, a key-type byte and the key; base32; 52 characters full, 24 compact. | PROTOCOL.md 10 |
| 4 | Contact card format | A signed statement of an identity's endpoint set at an epoch. With the one endpoint that version 1 allows: 139 bytes, or 155 with an invitation. Text form `MONOLITH1:` plus base32. | PROTOCOL.md 11 |
| 5 | Invitation capability | 16 random bytes in the card. Three modes; invitation-only by default; up to 16 valid at once; revocable; mismatches are dropped with no distinguishable reply. | PROTOCOL.md 12 |
| 6 | Duplicate-session resolution | After confirmation only. Same initiator: newer wins. Different initiators: the session initiated by the smaller identity key is preferred; if it is the older one it is probed first and loses if it is dead. | PROTOCOL.md 14 |
| 7 | Maximum sizes and budgets | One table per category; worst-case memory about 119 MiB. | RESOURCE_LIMITS.md |
| 8 | Rekey and reconnect thresholds | 24 hours, 2^32 frames or 2^40 bytes per direction; then a new handshake. No in-band rekey. | CRYPTOGRAPHY.md 7 |
| 9 | Tails control path | `amnesia` process to onion-grater on `127.0.0.1:951`, matched by executable path and user, to Tor's control port 9052; service target `127.0.0.1:29170`. | PLATFORM_TAILS.md 3 |
| 10 | Whonix profile and port mapping | Opt-in profile merged on the Gateway; `ADD_ONION` target rewritten to `{client-address}:29170`; listener bound to the Workstation's internal IPv4 address; connections accepted from the Gateway address only, by Monolith and by a source-restricted firewall rule. Untested. | PLATFORM_WHONIX.md 4 to 6 |
| 11 | Local storage encryption | Vault: Argon2id, wrapped random key, XChaCha20-Poly1305, atomic replace. Message store: open, SQLCipher recommended. | STORAGE.md, ADR 0005 |
| 12 | History off by default on ordinary Linux and Whonix | Yes, off by default on every platform. | STORAGE.md 1 |
| 13 | GUI framework | Undecided. Leaning GTK 4; Qt 6 is the runner-up. | ADR 0006 |

## 2. Changes to the initial proposals

Each item is something that was proposed or assumed at the start and turned
out to be wrong, outdated, inconsistent or better done differently.

Tor and platforms

1. Tails control ports. The filtered control port on Tails is 951 and
   Tor's own is 9052, which the live user cannot reach. 9051 is not used.
2. "Tails blocks ordinary Internet connectivity." Outdated since Tails 7.9:
   outgoing TCP of the live user is redirected into Tor, and connections to
   private address ranges go out directly. Monolith's guarantees therefore
   cannot lean on the Tails firewall, and "no clearnet fallback" has to be
   proven elsewhere (TEST_PLAN.md 11).
3. "Integrate through the supported Tails filtering architecture." There is
   no supported way for third-party software to get a control port profile
   on Tails. It takes root in every session, or inclusion in Tails, which
   requires being in Debian first.
4. "A dedicated onion-grater policy" on Whonix. Whonix merges all enabled
   profiles into one filter for every Workstation on the Gateway. A profile
   only adds permissions; per-application or per-host restriction does not
   work there.
5. Whonix recommends listening on `0.0.0.0`. Monolith binds one address
   instead.
5a. Opening the listener port with `EXTERNAL_OPEN_PORTS`. On non-Qubes
   Whonix that opens it to every Workstation on the Gateway, which Whonix
   documents as a feature. Monolith checks the source address itself and
   uses a firewall rule restricted to the Gateway. A compromised
   Workstation on the same segment can still impersonate the Gateway; only
   Qubes-Whonix or a Gateway of its own closes that.
6. Proof-of-work capability detection. Tor does not report whether the
   proof-of-work module is compiled in, and `ADD_ONION` succeeds without
   it. Only the version can be checked. The keyword exists from 0.4.9.2,
   not 0.4.8; Monolith requires 0.4.9.5 and always sends it.
7. `GETINFO` for bootstrap status. Not used. `status/bootstrap-phase` can
   carry guard or bridge addresses and is rewritten by the filters; Monolith
   asks only `status/circuit-established`. `onions/current` is not needed.
8. Ephemeral services with a discarded key. Not used. A service dies with
   its control connection, so the key is kept in memory to re-create the
   same service after a reconnect.
8a. Upload confirmation through `HS_DESC` events. Not used on Tails and
   Whonix. The events cover every Onion Service of the Tor instance, so a
   filter rule that passes them leaks other applications' or Workstations'
   addresses.
9. Tor's manual says control-port Onion Services are unsupported with the
   syscall sandbox, and Tails enables the sandbox. OnionShare on Tails
   works this way regardless. Recorded as something to verify first.
10. SOCKS isolation credentials. One token per contact for the life of the
    process, in Tor's `<torS0X>0` format, instead of per-session
    credentials.

Protocol and cryptography

11. BLAKE2s. Replaced by SHA-256 (ADR 0002).
12. `EndpointUpdateV1` with `previous_epoch`. An endpoint update is a
    contact card with a greater epoch; the extra field adds no protection.
13. File chunk of up to 64 KiB in a frame of up to 64 KiB. Cannot fit. The
    frame maximum is 64528 bytes and a chunk carries 64490.
14. Handshake record of up to 8 KiB. The handshake records are fixed at 32,
    96 and 64 bytes.
15. Contact card of up to 8 KiB. It is exactly 139 or 155 bytes in version 1.
15a. One endpoint per identity. The card now states a set of endpoints with
    a count, limited to one in version 1, so that rotation with overlap,
    migration, temporary and per-contact endpoints do not need a new
    identity model later.
16. Suggested text limits were lowered: chat 16 KiB, introduction 512
    bytes, profile text 1 KiB (RESOURCE_LIMITS.md 2).
17. `ContactReject`. Not a message. Rejection is silent so that it cannot be
    told from a pending request.
18. Message list. `Profile` and `FileAbort` were added; both are needed by
    the rest of the design.
18a. Blocked peers. Closing a blocked peer's connection at once would tell
    it that it is blocked. A blocked identity is treated like an unknown
    one.
18b. Contact state. Both sides confirm the contact relationship at the
    start of every session, so a contact-only message is never sent to a
    peer that no longer holds the sender as a contact.
19. File digest. Mandatory, carried in `FileComplete` and computed while
    streaming, instead of optional in the offer. SHA-256 instead of BLAKE3,
    to avoid one more primitive.
20. Contact card prefix. `MONOLITH1:` in upper case with base32, so that a
    QR code can use alphanumeric mode.

Dependencies and tooling

21. `OsRng` no longer exists in the current `rand_core` and `getrandom`;
    the type is `SysRng`.
22. `minicbor` does not enforce canonical decoding, which is why CBOR was
    not chosen.
23. `snow` has no audit and does not zeroize. Recorded as a risk of every
    Noise option; the session library is not chosen.
23a. The prologue binding was described as if it kept probers out. It does
    not: the identity key is public data. It is now described as
    opportunistic probing resistance.
23b. `ring` was banned in `deny.toml`. The ban is removed; a cryptographic
    library is not excluded for containing C or assembly.
24. Toolchain. The minimum supported Rust version is 1.85.1 and is separate
    from the developer toolchain, which is pinned at 1.95.0 for rustfmt and
    clippy. CI also builds with the current stable compiler.
25. The 2015 TorChat analysis is a Master's thesis by Rain Viigipuu titled
    "Security Analysis of Instant Messenger TorChat". Its highest-ranked
    finding is the missing contact authorization, not the weak cookie.

## 3. Open questions

Nothing below is settled. Each is described in the document named.

| Id | Question | Document |
| --- | --- | --- |
| Q1 to Q10 | Choice of session construction, responder identity disclosure, the prologue precondition, the proof input, XX or NN, certificate lifetime, practicality of TLS with raw public keys, binding the onion key, session limits, crypto back end | CRYPTOGRAPHY.md 11, ADR 0002 |
| P1 | Padding block size | PROTOCOL.md 17 |
| P2 | Bind the dialed onion key in the proof | PROTOCOL.md 17 |
| P3 | Epoch after restoring a backup | PROTOCOL.md 17 |
| P4 | Invisible declines cause indefinite retries | PROTOCOL.md 17 |
| P5 | Keep profile text in version 1 | PROTOCOL.md 17 |
| P6 | Message ordering across reconnects | PROTOCOL.md 17 |
| P7 | Display names when two builds have different Unicode tables | PROTOCOL.md 17 |
| C1 | `MaxStreams` value and semantics | TOR_CONTROL_SURFACE.md 6 |
| C2 | Proof-of-work queue parameters | TOR_CONTROL_SURFACE.md 6 |
| C3 | Confirming reachability on Tails and Whonix without `HS_DESC` | TOR_CONTROL_SURFACE.md 6 |
| T1 to T5 | Tails: the experimental preconditions (sandbox, OnionShare's path, required profile, packaging), profile matching, AppArmor, namespaces, Debian packaging. T1 blocks Phase 3 and Phase 6. | PLATFORM_TAILS.md 3.4, 7 |
| W1 to W7 | Whonix: profile test, Qubes addressing, the two-Workstation isolation test (blocks Phase 7), upstreaming the profile, SocksPort choice, a supported per-source port opening, KVM network design | PLATFORM_WHONIX.md 9 |
| ST1 to ST6 | Message store, locking, previous generation, permission checks, passphrase policy, Argon2id defaults (benchmark pending) | STORAGE.md 9 |
| A3, A4 | Configuration format, CLI parser | ARCHITECTURE.md 13 |
| - | GUI toolkit | ADR 0006 |
| - | Security contact address and key | SECURITY.md |

## 4. Proposed dependencies

Crates that are in use are recorded in `DEPENDENCIES.md` with what was
checked before they were added: `ed25519-dalek` 3.0.0, `sha2` 0.11.0,
`subtle` 2.6.1, `unicode-normalization` 0.1.25, and `proptest` for tests. The table below
lists what was considered at the start; for crates not yet in use the final
choice is made in the phase that first needs them.

| Crate | Version | For | Phase |
| --- | --- | --- | --- |
| `ed25519-dalek` | 3.0.0 (decided) | identity signatures | 1 |
| `sha2` | 0.11 (decided) | fingerprint, file digest | 1 |
| `sha3` | 0.11 or 0.12, decided with the Tor backend | onion address checksum | 3 |
| `data-encoding` | 2.11 | base32, base64 | 1 |
| `unicode-normalization` | 0.1.25 | NFC for names and passphrases | 1 |
| `getrandom` | 0.3 or 0.4 | CSPRNG | 1 |
| `zeroize`, `subtle` | 1.9, 2.6 | secret handling | 1 |
| `snow` or `rustls` | not chosen | session layer (ADR 0002) | 2 |
| `tokio` | 1.53 (LTS) | runtime | 3 |
| `tracing` | 0.1.44 | logging | 3 |
| `argon2`, `chacha20poly1305`, `hkdf` | 0.6, 0.11 | vault | 4 |
| `rusqlite` with SQLCipher | to be decided | message store | 4 |
| `qrcodegen` | 1.8.0 | QR codes | 8 |
| `gtk4` | 0.10 | desktop | 8 |
| `proptest` | 1.11 | property tests | 1 |
| `libfuzzer-sys`, `arbitrary` | 0.4, 1.4 | fuzzing | 1 |

The cryptographic crates are those of the current RustCrypto and dalek
generation; `DEPENDENCIES.md` section 2 records the decision. `snow` 0.10
depends on the previous generation, and mixing the two would put two
versions of the same cryptographic crate into the binary, which
`deny.toml` forbids. ADR 0002 has to resolve that if it selects `snow`.

Not used: `anyhow` in libraries, `serde` for anything signed or on the wire,
any HTTP client, any resolver. A session library and its crypto back end
are not chosen; see ADR 0002.
