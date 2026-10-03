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

Implemented in Phase 2: the session layer. Noise XK with a transport key
certified in the contact card, through `snow` with a resolver over the
current crates (ADR 0002); the contact card with the transport key; the
handshake, the encrypted frames and the session limits, in the crate
`monolith-session`. A caller without the contact card gets no reply from
the handshake; with it, it learns that the key holder is live. Public keys
are never access control. The rules that bind the handshake to identities
are Monolith's own and have no external review (F-R2). No network I/O, no
Tor and no storage encryption are part of Phase 2.

Provisional, and not to be built on yet:

| Area | State | Decided where |
| --- | --- | --- |
| Tails Onion Service integration | blocked on experiments on a current Tails | PLATFORM_TAILS.md 3.4 |
| Whonix isolation and firewall integration | measures specified, untested | PLATFORM_WHONIX.md 4.5 |
| Vault KDF parameters | proposed, benchmark pending | STORAGE.md 3.3 |
| Padding block size | mechanism decided, value not | ADR 0003 |
| Message store | open | STORAGE.md 5 |
| GUI toolkit | open | ADR 0006 |

Decided in the review: the project is MIT licensed; the minimum supported
Rust version is 1.85.1 and is separate from the developer toolchain; an
identity has a set of endpoints, limited to one in version 1.

Decided in the review of Phase 1:

- The copyright line of the MIT licence is "Copyright (c) 2026 Monolith
  contributors". It is the intended line, not a stand-in.
- Key separation is a protocol invariant: a contact card whose endpoint is
  the identity key is invalid (PROTOCOL.md 11.2, S33).
- A public key is valid only if it decompresses, is torsion-free and is
  not of small order (PROTOCOL.md 10.1). This is stricter than signature
  verification, on purpose.
- `subtle` is a direct dependency for comparing invitation capabilities.
- The cryptographic crates are those of the current generation:
  `ed25519-dalek` 3, `curve25519-dalek` 5, `sha2` 0.11
  (DEPENDENCIES.md 2).
- `native-tls` and `openssl-sys` stay banned as dependency control; ADR
  0002 may reconsider the second.
- Normalization is not part of protocol validity (PROTOCOL.md 9). P7 is
  closed.
- The order of the identity proofs is not decided in Phase 1. It is Q11 of
  ADR 0002.

Decided at the start of Phase 2, in ADR 0002:

- The session layer is `Noise_XK_25519_ChaChaPoly_SHA256`. Every identity
  has an X25519 transport key, generated independently of its other keys,
  which is the Noise static key.
- The contact card states the transport key and is the certificate for
  it. The responder's identity key is in the Noise prologue. The
  initiator presents its card in the third handshake message.
- The responder is authenticated first. No signature is made during a
  session, and there is no identity proof message; Q11 is closed.
- A card older than the newest one held of that identity, pinned or
  pending, does not open a contact session.
- A new transport key is a new card epoch. There is no overlap period and
  no revocation in version 1.
- TLS 1.3 with raw public keys was the other finalist and was not chosen.
- The Noise library is `snow` 0.10.0. Its primitives come from a resolver
  in the session crate over `x25519-dalek` 3, `chacha20poly1305` 0.11 and
  `sha2` 0.11, so that the transport key is held in a type that clears
  it and the key checks of rule F5 sit in one place (F-R1).

## 1. Answers

| # | Question | Answer | Where |
| --- | --- | --- | --- |
| 1 | Noise XX with an Ed25519 transcript proof, or TLS 1.3 | Neither. Noise XK with an X25519 transport key that the identity certifies in its contact card. ADR 0002 compares it with Noise XX and Noise NN with a transcript proof and with TLS 1.3 with raw public keys. A cryptographer's review of the binding rules is still wanted. | ADR 0002, CRYPTOGRAPHY.md |
| 2 | Canonical wire encoding | Fixed-layout binary, hand-written encoders and decoders, no serialization framework. One valid encoding per structure. | ADR 0003, PROTOCOL.md 2, 5 |
| 3 | Fingerprint encoding | SHA-256 over a prefix, a key-type byte and the key; base32; 52 characters full, 24 compact. | PROTOCOL.md 10 |
| 4 | Contact card format | A signed statement of an identity's transport key and endpoint set at an epoch. With the one endpoint that version 1 allows: 171 bytes, or 187 with an invitation. Text form `MONOLITH1:` plus base32. | PROTOCOL.md 11 |
| 5 | Invitation capability | 16 random bytes in the card: a reusable bearer capability that allows a contact request and authenticates nobody. Three modes; invitation-only by default; several cards per identity, one per capability; up to 16 valid at once; revocable for new requests only; mismatches are dropped with no distinguishable reply. Section 6. | PROTOCOL.md 12 |
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
14. Handshake record of up to 8 KiB. The handshake messages are fixed at
    48, 48 and 235 bytes.
15. Contact card of up to 8 KiB. It is exactly 171 or 187 bytes in version 1.
15a. One endpoint per identity. The card now states a set of endpoints with
    a count, limited to one in version 1, so that rotation with overlap,
    migration and temporary endpoints do not need a new identity model
    later. Per-contact endpoints are not covered: they need a
    recipient-bound card format (section 8.2, C-F).
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
23. `snow` was described as unaudited. It was audited in 2024; the finding
    that it does not clear keys is still open. It is the session library,
    used without its own crypto (ADR 0002, F-R1).
23a. The prologue binding was described as if it kept probers out. It does
    not: the keys a caller needs are public data. It is described as
    probing resistance, never as access control.
23c. An identity proof signed inside the channel, a preamble with a version
    and feature bits. All three are gone: the handshake authenticates a
    transport key that the contact card certifies, and the version is a
    label in the handshake prologue.
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
| F-R2 to F-R6 | Open points of the session layer: no external review of the binding rules and of the rotation and retirement state machine, one maintainer of the library, no revocation of transport keys, stale cards towards parties without a record, no binding to the onion address | ADR 0002 |
| P1 | Padding block size | PROTOCOL.md 17 |
| P2 | Bind the session to the dialed onion key | PROTOCOL.md 17 |
| P3 | Epoch after restoring a backup | PROTOCOL.md 17 |
| P4 | Invisible declines cause indefinite retries | PROTOCOL.md 17 |
| P5 | Keep profile text in version 1 | PROTOCOL.md 17 |
| P6 | Message ordering across reconnects | PROTOCOL.md 17 |
| P8 | A responder that answers two transport keys at once during a rotation (the bounded overlap itself is decided) | PROTOCOL.md 17 |
| CR-1 | Timing of a rotation: when the identity switches to the new key, and how long the old one is kept | DESIGN_QUESTIONS.md 8.2 |
| CR-2 | A recipient-bound card format for per-contact endpoints | DESIGN_QUESTIONS.md 8.2 |
| CR-3 | A bound on how far an epoch may jump in one card | DESIGN_QUESTIONS.md 8.2 |
| P9 | Whether the card in a ContactRequest is still needed | PROTOCOL.md 17 |
| - | The value of `MAX_ACTIVE_INVITATIONS` | RESOURCE_LIMITS.md 5 |
| MI-1 to MI-4 | Several local identities: a target port per identity on Tails and Whonix, budget values and the number of identities, mixed storage modes, the phase that offers several in the interface | DESIGN_QUESTIONS.md 7 |
| C1 | `MaxStreams` value and semantics | TOR_CONTROL_SURFACE.md 6 |
| C2 | Proof-of-work queue parameters | TOR_CONTROL_SURFACE.md 6 |
| C3 | Confirming reachability on Tails and Whonix without `HS_DESC` | TOR_CONTROL_SURFACE.md 6 |
| T1 to T5 | Tails: the experimental preconditions (sandbox, OnionShare's path, required profile, packaging), profile matching, AppArmor, namespaces, Debian packaging. T1 blocks Phase 6. | PLATFORM_TAILS.md 3.4, 7 |
| W1 to W7 | Whonix: profile test, Qubes addressing, the two-Workstation isolation test (blocks Phase 7), upstreaming the profile, SocksPort choice, a supported per-source port opening, KVM network design | PLATFORM_WHONIX.md 9 |
| ST1 to ST7 | Message store, locking, previous generation, permission checks, passphrase policy, Argon2id defaults (benchmark pending), storage mode per identity | STORAGE.md 9 |
| T3-7 | Phase 3: the two-node test on a private Tor network is written but has not run; no Tor in the development environment | tests/tor-network/README.md |
| A3, A4 | Configuration format, CLI parser | ARCHITECTURE.md 13 |
| - | GUI toolkit | ADR 0006 |
| - | Security contact address and key | SECURITY.md |

## 4. Proposed dependencies

Crates that are in use are recorded in `DEPENDENCIES.md` with what was
checked before they were added: `ed25519-dalek` 3.0.0, `sha2` 0.11.0,
`subtle` 2.6.1, and `proptest` for tests. The table below
lists what was considered at the start; for crates not yet in use the final
choice is made in the phase that first needs them.

| Crate | Version | For | Phase |
| --- | --- | --- | --- |
| `ed25519-dalek` | 3.0.0 (decided) | identity signatures | 1 |
| `sha2` | 0.11 (decided) | fingerprint, file digest | 1 |
| `sha3` | 0.11 or 0.12, decided with the Tor backend | onion address checksum | 3 |
| `data-encoding` | 2.11 | base32, base64 | 1 |
| `unicode-normalization` | 0.1.25 | NFC for the vault passphrase only; not used by the protocol | 4 |
| `getrandom` | 0.3 (decided) | CSPRNG | 2 |
| `zeroize`, `subtle` | 1.9, 2.6 | secret handling | 1 |
| `snow`, `x25519-dalek`, `chacha20poly1305` | 0.10.0, 3.0.0, 0.11.0 (decided) | session layer (ADR 0002) | 2 |
| `tokio` | 1.53 (LTS) | runtime | 3 |
| `tracing` | 0.1.44 | logging | 3 |
| `argon2`, `chacha20poly1305`, `hkdf` | 0.6, 0.11 | vault | 4 |
| `rusqlite` with SQLCipher | to be decided | message store | 4 |
| `qrcodegen` | 1.8.0 | QR codes | 8 |
| `gtk4` | 0.10 | desktop | 8 |
| `proptest` | 1.11 | property tests | 1 |
| `libfuzzer-sys`, `arbitrary` | 0.4, 1.4 | fuzzing | 1 |

The cryptographic crates are those of the current RustCrypto and dalek
generation; `DEPENDENCIES.md` section 2 records the decision. The stock
resolver of `snow` 0.10 depends on the previous generation. Monolith does
not use it; ADR 0002, F-R1, says why.

Not used: `anyhow` in libraries, `serde` for anything signed or on the wire,
any HTTP client, any DNS resolver.

## 5. Phase 3 design review

Before any Tor code was written, the Phase 0 documents were compared with
the Phase 3 brief and with the current Tor specifications and source
(sources at the end of this section, accessed 2026-10-02). Where they
disagreed, the disagreement was recorded here before anything was
implemented. Section 5.1 lists the six points that needed a decision,
with the decision taken on 2026-10-02; section 5.2 lists changes that the
brief decides and that reduce the surface.

### 5.1 Decisions

T3-1. The Tails precondition.
    `ARCHITECTURE.md` section 12, ADR 0004 and `TOR_INTEGRATION.md` make
    the checks of `PLATFORM_TAILS.md` section 3.4 a precondition of Phase 3:
    whether `ADD_ONION` works through onion-grater while Tails runs Tor
    with `Sandbox 1`. None of them has been made. The Phase 3 brief
    excludes Tails work and asks for a generic System Tor backend.
    - (a) Make the Tails checks first, on a current Tails release. Phase 3
      waits for a Tails machine.
    - (b) Lift the precondition for the generic `SystemTorBackend` and
      keep it for Phase 6. If `ADD_ONION` turns out not to work on Tails,
      what changes is how a Tails build publishes its service (or that it
      cannot, which `PLATFORM_TAILS.md` already allows as an honest
      limitation). Outbound SOCKS, the control parser, authentication and
      the `TorBackend` contract are not affected, because the trait
      already hides how publication is done.
    - Recommendation: (b). The ARCHITECTURE and ADR text is changed to say
      that the Tails checks gate Phase 6 and any claim of Tails support,
      not the generic backend.
    - Decided: (b).

T3-2. The `ADD_ONION` forms and the key modes.
    `TOR_CONTROL_SURFACE.md` allows exactly two forms, `NEW:ED25519-V3`
    with the key returned and `ED25519-V3:<key>`, and never sends
    `DiscardPK`, so that an ephemeral endpoint can be created again from
    the key held in memory after a control connection is lost. The two
    onion-grater profiles in `integrations/` match these two forms
    character for character. The brief asks for three modes: A, a new key
    with `DiscardPK`; B, a stored key; C, a new key returned to the caller.
    - (a) Keep the two forms. Mode C is the first form, mode B the
      second. An ephemeral endpoint is mode C with the key kept in memory
      only and never written. Mode A is not offered.
    - (b) Add mode A as a third form. Monolith then holds no onion key at
      all for such an endpoint, but a lost control connection or a Tor
      restart ends the endpoint for good: every contact holds a signed card
      that names it, and they cannot reach the user until a card with a
      greater epoch reaches them. Both platform profiles need a third
      pattern.
    - (c) Replace the Phase 0 ephemeral endpoint by mode A.
    - Recommendation: (a). The key of mode A is not in Monolith's memory
      but it is in Tor's, so the gain is small, and the cost is an
      endpoint that dies with any Tor restart while contacts still hold it.
      Mode A can be added later as a third form without changing the
      `TorBackend` contract.
    - Decided: (a). The two forms differ only in the key and share one
      publication policy: `Flags=MaxStreamsCloseCircuit`, `MaxStreams=8`,
      `PoWDefensesEnabled=1`, and none of `DiscardPK`, `Detach`,
      `NonAnonymous`, `ClientAuth`, `ClientAuthV3` or custom proof-of-work
      queue parameters.

T3-3. Which status Monolith reads.
    Phase 0 reads only `GETINFO status/circuit-established` and
    deliberately not `status/bootstrap-phase`, because the warning form of
    that reply carries the identity and address of a guard or bridge
    (`HOSTID=`, `HOSTADDR=`). The brief asks Monolith to tell a Tor that is
    bootstrapping from one that is reachable but not ready, and from one
    that is ready. The specification guarantees only the tags `starting`
    and `done`; percentages and the order of phases are not stable.
    - (a) Keep `status/circuit-established` alone. "Bootstrapping" and
      "not ready" are then one state.
    - (b) Add `status/bootstrap-phase`. The parser keeps the `PROGRESS`
      number and whether `TAG` is `done`, and drops the rest of the line
      while parsing, so that no guard or bridge address is stored, logged
      or shown. Readiness still comes from `status/circuit-established`.
      On platforms with a filter the key is refused and the progress is
      shown as unknown.
    - Recommendation: (b). The version comes from `PROTOCOLINFO`, so
      `GETINFO version` is not needed. `network-liveness` adds nothing that
      the two keys do not.
    - Decided: (b), with the line filtered as strictly as described: the
      progress number and `done`, nothing else survives the parser.

T3-4. Stream isolation policy.
    Phase 0: one random 16-byte token per contact, made at process start
    and kept in memory, sent as `<torS0X>0` with the token in hex as the
    password. The brief proposes a fresh token per session.
    - Per session: every session to a contact needs a new rendezvous
      circuit, so each reconnect costs a full introduction (and proof of
      work when the contact's service is under attack), and the
      introduction points of the contact see more traffic. Two sessions to
      the same contact cannot be linked by circuit reuse; the contact
      itself links them anyway through the handshake.
    - Per contact: reconnects may reuse a rendezvous circuit. Streams to
      different contacts never share a circuit, because a rendezvous
      circuit belongs to one Onion Service, and the token keeps Monolith's
      streams apart from other applications on a shared SocksPort.
    - In both cases the token is random and names nothing: not the
      identity, the onion address, a name or a fingerprint. The backend
      takes the token from its caller and has no policy of its own.
    - Recommendation: per contact, as in Phase 0.
    - Decided: per contact, reused for reconnects to that contact, a
      different token for every contact. The token comes from the
      operating system's CSPRNG, contains nothing identifying, is not
      logged and is not exposed outside the Tor adapter. It is runtime
      state, not contact data, and is made again after a restart. The
      structured `<torS0X>0` format is used, not legacy username and
      password isolation. Isolation is not claimed to prevent traffic
      correlation.

T3-5. Proof of work.
    Phase 0 sends `PoWDefensesEnabled=1` on every `ADD_ONION`, with Tor's
    default queue parameters. The brief asks that it not be enabled
    casually. Facts: the keywords exist from tor 0.4.9.2-alpha, so every
    Tor that Monolith accepts knows them; with no attack the required
    effort is zero; the defense needs a tor built with GPL code, and
    `ADD_ONION` succeeds without it, so Monolith cannot tell whether it is
    active and does not claim it is.
    - Recommendation: keep the Phase 0 decision. It costs nothing when
      there is no attack and was reviewed in Phase 0. Queue parameters stay
      at Tor's defaults (open item C2).
    - Decided: `PoWDefensesEnabled=1` on both forms, no `PoWQueueRate` or
      `PoWQueueBurst`. Requested is not verified active; Monolith does not
      run the tor binary to find out. Application limits stay mandatory.

T3-6. `MaxStreams`.
    Phase 0 sends `MaxStreams=8` with `MaxStreamsCloseCircuit` and leaves
    the value open (C1): the specification does not say whether Tor counts
    streams open at the same time or all streams a circuit ever carried.
    One Monolith session uses one stream, so either reading leaves room for
    reconnects on one circuit, and closing a circuit only forces a new one.
    - Recommendation: keep 8 as a provisional value, documented as defense
      in depth behind the application budgets, and settle it with the
      resource tuning of Phase 4.
    - Decided: `MaxStreams=8` with `MaxStreamsCloseCircuit` on both forms,
      provisional. It applies per rendezvous circuit, not to the service
      as a whole, and is reviewed in Phase 4 against the real connection
      model and measurements.

### 5.2 Changes the brief decides

These narrow what Phase 0 allowed. They are applied with the
implementation and recorded in the documents they change.

- No `SETEVENTS` at all. Phase 0 used `HS_DESC` events on Linux without a
  filter to confirm the descriptor upload; those events name every Onion
  Service of the Tor instance, including other applications'. A service
  is shown as published once `ADD_ONION` succeeded; confirming that it is
  reachable stays open (C3).
- Control authentication: SAFECOOKIE only, for a control endpoint the user
  configured. COOKIE is not implemented (the specification marks it
  deprecated). HASHEDPASSWORD is deferred until there is a need. NULL is
  used only when the configuration says that the endpoint is a trusted
  filter, never because a server offers it.
- `connect_onion` takes no port. The virtual port is the protocol constant
  `ONION_VIRTUAL_PORT` (29170), so the backend cannot be used as a
  general SOCKS client.
- Tor versions. The feature baseline is 0.4.9.5: the first stable release
  with the proof-of-work keywords of `ADD_ONION` (0.4.9.2-alpha) and the
  `<torS0X>0` isolation format (0.4.9.1-alpha). 0.4.9.13 or later is
  recommended, for TROVE-2026-053, and only the 0.4.9 series is supported
  upstream. The version comes from `PROTOCOLINFO` of the local Tor;
  nothing is fetched from the network.
- `ClientAuth=` is never sent. Current tor accepts the keyword but no
  longer handles it.
- The brief assumes an existing onion address validator. There is none
  yet: `monolith-identity` validates the 32-byte key, and the `.onion`
  name with its SHA3-256 checksum was left to the phase that needs it.
  Phase 3 adds the conversion between a ServiceID and an
  `OnionServiceKey`, with the `sha3` crate, and every ServiceID Tor
  returns goes through it.

### 5.3 Errata noticed in the Tor specification

For the record, not acted on:

- The `ADD_ONION` grammar has a stray CRLF before `ClientAuthV3`, and
  `PoWQueueBurst=` is followed by the misspelled `PowQBurt`.
- The SOCKS extensions say extended error codes "can be disabled"; in
  tor they are off unless a SocksPort has the `ExtendedErrors` flag.
- `status/bootstrap-phase` names `HOST`; tor emits `HOSTID`.
- The `ADD_ONION` reply grammar omits the `ClientAuthV3` lines that tor
  sends and that the examples show.

### 5.4 Sources

Accessed 2026-10-02. Tor source and specification at torspec commit
928c0c0 (2026-09-30) and tor commit c6c6170 (2026-09-29).

- https://spec.torproject.org/control-spec/commands.html
- https://spec.torproject.org/control-spec/message-format.html
- https://spec.torproject.org/control-spec/replies.html
- https://spec.torproject.org/control-spec/implementation-notes.html
- https://spec.torproject.org/socks-extensions.html
- https://spec.torproject.org/address-spec.html
- https://spec.torproject.org/rend-spec/encoding-onion-addresses.html
- https://spec.torproject.org/proposals/193-safe-cookie-authentication.html
- https://spec.torproject.org/intro/conventions.html
- https://gitlab.torproject.org/tpo/core/tor/-/blob/main/doc/man/tor.1.txt
- https://gitlab.torproject.org/tpo/core/tor/-/blob/main/ChangeLog
- https://gitlab.torproject.org/tpo/core/team/-/wikis/NetworkTeam/CoreTorReleases
- https://forum.torproject.org/t/security-release-0-4-9-13/22178
- https://blog.torproject.org/sunsetting-tor-048/
- https://www.rfc-editor.org/rfc/rfc1928.txt

## 6. Contact cards and invitation capabilities

On 2026-10-02 the semantics of the contact card and of the invitation
capability were made explicit. This is a clarification of the design of
Phase 0, not a change of the wire format: the card layout, the 16-byte
capability, its signing, the handshake, the Onion Service and the protocol
version are unchanged. `PROTOCOL.md` sections 12.2 and 12.3 hold the
rules; this section records what was decided.

- There is one contact card format. How a card is distributed (privately,
  in a directory, on a website, as a QR code) is the user's decision and
  not a protocol type.
- The capability is a bearer capability: an authorization to attempt a
  contact request, not a proof of identity. Authentication stays with the
  identity and the handshake.
- Whether a capability stays confidential depends on how the user
  distributes the card. A capability in a published card is public.
- Cards are reusable. A capability is valid until revoked; version 1 has
  no use counter, single-use capability or expiry.
- An identity may issue several cards at once that differ only in their
  capability and signature. They are one statement of the identity, not a
  conflict. A card may be made for one directory and withdrawn on its own.
- Removing a card from a directory is not revocation. Only local
  revocation stops new requests that carry its capability.
- The capabilities the user accepts form a bounded active set.
  `MAX_ACTIVE_INVITATIONS` stays at the Phase 0 value of 16 as a
  provisional bound, to be confirmed in Phase 4.
- Revocation removes a capability from the active set for new requests.
  A validly signed card does not show that its capability is accepted
  today. Revocation does not touch accepted contacts, sessions, keys, the
  Onion Service or history.
- A peer cannot tell an unknown, revoked or foreign capability from no
  capability: every such request gets the same Close.
- A card without a capability is an open card, and Monolith does not add
  one. Local labels of capabilities are never part of the card.
- The capability keeps its handling inside Monolith whether or not it was
  published: 16 bytes from the CSPRNG, not `Copy`, redacted `Debug`,
  constant-time comparison, erased on drop, not logged.

Consequences recorded with this clarification:

- The request mode applies to the identity, not to a card, because a
  request does not say which card it came from.
- A capability is not tied to an epoch. After a change of transport key
  or endpoints the user issues new cards, which may carry the same
  capabilities.
- Revocation needs no local state beyond the active set and its labels.
  A revoked capability is removed, not remembered.

Decided in the owner's review the same day:

- P10. A card imported by hand that differs from the held card only in
  its capability replaces it, for a peer that is not an accepted contact.
  A capability-only difference is not an identity, endpoint, transport
  key or epoch change. For an accepted contact, changing or revoking the
  capability it was introduced with does not change the relationship.
- P11. Pending requests that passed admission stay when the capability
  that admitted them is revoked. Phase 4 may offer a separate local action
  that revokes a capability and discards the requests it admitted; a
  pending request may then record locally which capability admitted it.
  That record is never sent.
- The active set stays bounded at 16 (`MAX_ACTIVE_INVITATIONS`), a
  provisional Phase 4 resource limit. When it is full, creating another
  capability fails explicitly; no capability is ever revoked or evicted
  automatically.
- One identity may have cards with and without a capability at the same
  time. With public contact requests enabled (open mode) a capability is
  not needed for admission, so revoking one cannot keep a peer from
  submitting a request through the open path. With them disabled
  (invitation mode) an active capability is required from unknown peers,
  and revoking one controls admission. Directory cards that are revoked
  on their own are therefore most useful in invitation mode. No wire
  field or card type expresses this.

Implementation. The card and session code of Phases 1 and 2 already
follow these rules: `evaluate_card` ignores the capability, and a request
carries the capability of the card held of the peer. The active set,
revocation, the replacement of P10, the decision of `PROTOCOL.md`
section 12, step 3, and the tests T-INV-1 to T-INV-11 of `TEST_PLAN.md`
belong to the contact store of Phase 4.

## 7. Several local identities

On 2026-10-02 support for several local identities in one installation
was made an architectural requirement. Nothing of it is implemented as a
feature yet; the requirement is that no layer assumes exactly one local
identity, so that it can be added without a new wire protocol.
`ARCHITECTURE.md` section 1.1 describes the model, `STORAGE.md` section
1.1 the storage, `TOR_INTEGRATION.md` sections 3.2 and 4.3 the Tor side,
`RESOURCE_LIMITS.md` section 5.1 the budgets, `THREAT_MODEL.md` adversary
S what still links identities, and S39 to S46 the invariants.

Requirement and where it is held:

| Requirement | Invariant |
| --- | --- |
| A local identity has its own identity, transport and Onion Service keys | S39 |
| Contact state is scoped to one local identity | S40 |
| Invitation capabilities are scoped to one local identity | S41 |
| Stream isolation is scoped to local identity and contact | S42 |
| An inbound service maps to exactly one local identity | S43 |
| A peer of one identity learns nothing about the others | S44 |
| Every outbound connection names its local identity | S45 |
| No local identity is process-wide state | S46 |

Review of Phases 1 to 3:

- No wire dependency on one local identity. Cards, the handshake, frames,
  keys, the onion address format and the protocol version are unchanged.
- No global state. No static or thread-local value holds an identity, a
  key, a party, a card, a capability, a service or an isolation group.
- `TorBackend` takes no identity and keeps no identity state. Each
  `publish_onion` has its own control connection, listener and handle, so
  one `SystemTorBackend` can hold several publications at once; the mock
  backend does too.
- Isolation groups are made by the backend and kept by the caller; the
  scope per local identity and contact is the core's, in Phase 4.
- `link::dial` and `link::answer` take the local party, the record or
  lookup and the isolation group as arguments, and `serve` runs one
  service, so the accept loop of a service knows its identity.
- `Budgets` is a value passed by reference. Phase 3 uses one per process;
  Phase 4 splits the budgets into process-wide, per identity and per
  contact.
- `dev-chat` makes one identity per run. It is a test aid and stays so.
- No Phase 3 interface had to change. Structural tests were added for two
  publications on one backend and for two local identities on one
  backend.

Open items:

MI-1. The Tails and Whonix profiles fix the target port at 29170, so all
      services of a process would share one listener and a stream would
      not say which identity it addresses. Proposed: a small fixed range
      of ports in the profiles, one per identity; until then one identity
      at a time receives streams on those platforms. Phases 6 and 7.

MI-2. The values of the per-identity and process-wide budgets, and the
      number of local identities one process may hold. Phase 4.

MI-3. Whether ephemeral and persistent identities can be mixed in one
      process (`STORAGE.md` ST7). Phase 4.

MI-4. Which phase offers several identities in the interface. Not
      decided; Phase 4 builds identity-scoped storage either way.

Not a goal: unlinkability of identities that run in one process. They
share Tor, presence and the process (`THREAT_MODEL.md` adversary S).
Several identities are also not multi-device: one identity on several
devices needs device keys and is separate later work.

## 8. Credential binding review

On 2026-10-02 the authentication binding of Phases 2 and 3 was reopened
for six reported issues around rules F2 and F4. The session construction,
`Noise_XK_25519_ChaChaPoly_SHA256`, was not reconsidered. Each claim was
first checked against the code and tests of `phase-3-system-tor` at
`fe5327c`; section 8.1 records the result before anything was changed.

### 8.1 Verification of the reported issues

A. Outbound admission uses a stale record. Confirmed.
   `link::dial` (`crates/monolith-core/src/link.rs`) takes
   `record: PeerRecord<'_>` as an argument, then awaits a dial slot
   (`budgets.dial().await`), the Tor stream (`connect_onion`) and the
   three handshake messages, and only then calls
   `outbound.admit(record)`. The record is whatever the caller held when
   it called `dial`: either a snapshot, or a borrow that keeps the
   caller's contact state from changing for the whole dial. The session
   layer itself is correct for a fresh record:
   `a_dialed_card_that_was_superseded_meanwhile_gives_no_contact_session`
   (`crates/monolith-session/src/tests/contacts.rs`) passes a newer
   record to `OutboundPeer::admit` and gets `StaleCard`. No test drives
   the same case through `link::dial`, and `dev.rs` passes the card it
   dials as the record. `link::answer` is not affected: its lookup runs
   after message 3, with no await before `admit`.

B. A session keeps its standing after its transport key is superseded.
   Confirmed. The standing of an `AuthenticatedSession` is fixed in
   `AuthenticatedSession::new` and changes only through `block_peer`,
   `remove_contact`, `close` or the end of the stream. Nothing tells an
   open session that a newer card of its peer was recorded. F4 is applied
   only at admission (`PeerRecord::admit`); `PROTOCOL.md` sections 6.2,
   8.8 and 11.4 say nothing about sessions that are already open. A
   session authenticated with the old key stays in
   `AuthenticatedContact` and keeps delivering. The duplicate rule
   (`crates/monolith-protocol/src/duplicate.rs`) looks at identities and
   initiators only, so it could keep such a session over one made with
   the new key. No code calls the duplicate rule yet.

C. A local party can pair another identity's card with the local
   transport key. Confirmed, narrowed. `LocalParty::new(card, transport)`
   (`crates/monolith-session/src/key.rs`) checks that the card has no
   capability and states the public half of `transport`. It never sees
   the identity secret key, so it cannot check that the card is the local
   identity's. A card signed by Mallory that names Bob's transport key is
   accepted with Bob's transport secret: the test
   `the_local_party_of_a_session_is_fixed_by_its_card`
   (`crates/monolith-session/src/tests/handshake.rs`) builds exactly that
   party. Such a party answers handshakes with Mallory's identity in the
   prologue and Bob's key, and as an initiator presents Mallory's card
   with Bob's key, which passes F3 at the peer. It is not reachable by a
   peer: it needs local code to pair the wrong card with the key, and no
   production path does so today (`dev.rs` signs its own card). The type
   does not prevent it.

D. A higher-epoch card replaces the accepted credential at once.
   Confirmed. In `PeerRecord::admit`
   (`crates/monolith-protocol/src/session.rs`) `CardChange::Newer` keeps
   the standing of the record, and the caller records the card as the
   newest held. From then on F4 compares every card with it, so the
   previous transport key gets `StaleCard`. Nothing requires the newer
   card to have come through a session made with the current key.
   `a_newer_card_keeps_the_contact_and_is_reported_as_a_pending_change`
   and `a_retired_key_is_refused_as_soon_as_the_successor_card_was_shown`
   (`contacts.rs`) show both halves: whoever holds the identity key alone
   can sign a card with a new transport key, be accepted as the contact,
   and lock out the holder of the current key. `PROTOCOL.md` section 6.2
   (table, first row with a record) and section 11.4 specify this.

E. Simultaneous rotation deadlocks. Confirmed for the specified
   procedure. `PROTOCOL.md` section 11.4: an identity that replaces its
   transport key "answers only handshakes made with the new key", and
   "until then the two can talk only when the identity dials". When both
   sides switch before either successor card has reached the other, each
   dials the other's old key and gets no reply. Nothing in the code
   implements rotation; `LocalParty` holds one transport key.

F. Per-contact cards with independent epochs. Confirmed.
   `PROTOCOL.md` section 11.4 lists "endpoints given to one contact only.
   This needs no extra field: epochs only have to increase as seen by each
   receiver", and ADR 0001 says the same. Cards are bearer statements. A
   card made for Carol with epoch 6 is, for Bob who holds epoch 5, a newer
   card of the same identity under section 11.4. `THREAT_MODEL.md` and
   item 15a of section 2 only say that the set model leaves room for such
   endpoints, without the epoch claim.

None of the claims was wrong. The changes that follow keep the wire
format, the Noise pattern and suite, and the card layout.

### 8.2 What was decided and changed

The chain stays: the identity signs the transport credential (F1), Noise
XK proves possession of the transport key, and the session follows. What
changed is the meaning of the states in between. A valid signature by the
identity says that the identity issued a credential. The active
credential says which transport key is trusted for the contact today.
The two are not the same statement.

C-C (F2). F2 has two halves (`CRYPTOGRAPHY.md` section 5.2). A, the
    transcript: the responder's identity key in the prologue, unchanged.
    B, local-state integrity: `LocalParty::issue` signs the local card from
    the local identity key for the transport key that comes with it, and
    there is no constructor that takes a card (S47). The identity key is
    not kept in the party. Remote cards stay `ContactCard`; the local card
    exists only inside a `LocalParty`, so the two cannot be confused.

C-D (F4). `Credentials` holds per contact the active card, an authorized
    successor, a pending successor and the retired key
    (`PROTOCOL.md` section 11.4, S36, S48). A newer key becomes active only
    through continuity, an EndpointUpdate on a session of the active key
    followed by a handshake proving the new key, or through the user's
    confirmation of exactly that card. Other newer keys are pending: the
    session gets `Standing::PendingSuccessor`, treated like a stranger.
    Endpoint-only changes keep the transport credential and advance the
    active card; where to dial stays the user's decision and never changes
    which key is trusted. Manual import: for a requested contact the card
    replaces the held one (P10); for an accepted contact a new key is only
    pending, in place of any pending card a peer presented, and confirming
    it is a separate action. The continuity rules apply to requested
    contacts as well: the card the user imported is their active card, and
    a new key presented in a handshake is pending there too. A card stating the
    retired key never opens a contact session again.

C-A (fresh admission). `link::dial` and `link::answer` take an admission
    function called after the last wait; `PeerRecord` borrows the
    credentials mutably, so deciding the standing and recording the change
    are one call (S50). This is the smallest abstraction that gives fresh
    admission without a contact store. A Phase 4 store holds one lock or
    transaction around lookup, admission and keeping the withdrawal.

C-B (retirement). Promotion retires the previous key. Every session whose
    card no longer states the active key (`Credentials::authorizes`) is
    withdrawn: `AuthenticatedSession::withdraw` ends it with Close and
    delivers nothing more; each link has a `Withdrawal` kept with the
    session in the admission step, which stops delivery before the next
    buffered frame and wakes a waiting link (S49). The second race, a
    retirement right after a fresh admission, is closed by the two
    together: the admission step registers the withdrawal, and any later
    retirement uses it. The duplicate rule takes for each session whether
    its key is current and closes a non-current one before the preference
    is looked at.

C-E (rotation). The identity keeps the old key while it rotates: it
    announces the successor on sessions of the old key, proves the new key
    to each contact by dialing with it, and switches the key it answers
    with when its contacts have promoted it. P8 is revisited: the overlap
    is bounded and explicit, and a responder still answers one key at a
    time. Simultaneous rotation completes as long as both old keys answer
    until the successors were exchanged; if both disappear first, nothing
    is taken over and the recovery is an out-of-band card the user
    confirms. When to switch (CR-1) is set with the contact store.

C-F (per-contact endpoints). Withdrawn from `PROTOCOL.md` 11.4 and ADR
    0001. Version 1 cards are bearer statements under one epoch per
    identity; per-contact endpoints need a recipient-bound or namespaced
    signed format (CR-2), a later card version. No field was added and no
    wire byte changed.

Limits stated in the documents: a copied identity key alone yields only
pending successors; with the active transport key it yields continuity
and is indistinguishable from the identity, and no rule of F1 to F5 can
tell them apart (`THREAT_MODEL.md` adversary Q). Invitation capabilities
are untouched: cards that differ only in their capability are one
statement, and the capability is not part of the credentials.

Not changed: the Noise pattern and suite, the prologue, the card layout
and signature, the message set and the protocol version. Wire bytes and
the handshake vectors are the same; one fuzz seed was added for the new
standing.

Spec authority. The rule that the documents always win over the code is
replaced: the normative documents define intended behavior, `STATUS.md`
records what is implemented, and a disagreement is a bug to resolve in
either direction (`ARCHITECTURE.md` section 9).

### 8.3 The two review passes

Two focused reviews were made of the result: one of the authentication
and credential semantics, one of the implementation, state and
concurrency. Fixed after them:

- Crossing dials of two rotations deadlocked. When both identities rotate
  and their first dials with the new keys cross, each promotes the other's
  new key and withdraws the session it opened to the other's old key, so
  no session of a new key was ever confirmed, which was the trigger to
  start answering with the new key. The procedure of `PROTOCOL.md` 11.4
  now answers with the new key from the first use on, and dials a known
  authorized successor first; a test makes the crossing dials.
- Withdrawing stale or pending sessions at admission told the peer that
  it is held as a contact. Only sessions admitted as a contact's are
  withdrawn; the others stay on the path of a stranger.
- A presented card could displace a pending card the user imported. It
  no longer can.
- A card with the active key at the epoch of the authorized successor
  silently dropped the successor. It is a conflict now.
- An initiator sent message 3, with its identity and card, before it
  admitted the responder. It now admits after message 2 and does not send
  message 3 to a key that was retired or is only pending
  (`PROTOCOL.md` 4.4, check 3).
- The outbound request took its capability from the dialed card and the
  inbound one from the record. Both take it from the record now (P10).
- A link that had reported its withdrawal could wait for the idle limit
  on the next call; every call now fails at once. Links report when they
  end, so a store can forget their withdrawals, also when `answer` refused
  a peer after the admission. The withdrawal is handed out only to the
  admission function. The Close and the shutdown of a withdrawn link are
  bounded.
- Documented: only the key retired last is remembered; an announced
  successor does not expire; an endpoint change during a rotation is a
  new successor card; what a returned message changes in the contact state
  is applied only while its session still stands; the admission function
  must not block; credentials are persisted before the result is used.

Not changed, and why:

- `Link::send` is not cancel-safe: a send dropped in the middle of a
  write leaves part of a frame on the stream. That predates this review
  and ends the session at the peer.
- CR-3, a bound on epoch jumps, is a protocol rule and is left to the
  owner. Without it, a card the user imports with the active key and an
  epoch near the largest makes every later card of the owner stale, and
  a presented card with such an epoch occupies the pending slot until the
  user imports another. Recommended: refuse a card whose epoch exceeds
  the held one by more than 2^48, which leaves room for the jump of 2^32
  that P3 proposes for a restored backup.

### 8.4 Commits

On `phase-3-system-tor`, after `fe5327c`, oldest first:

| Commit | Content |
| --- | --- |
| `3bc40c5` | Section 8.1, the verification of the reported issues. |
| `5e21a4f` | `LocalParty::issue`; no constructor takes a card (F2, half B). |
| `994fa23` | `Credentials`: active, authorized, pending, retired. |
| `2792aa7` | `PeerRecord` borrows the credentials; `Standing::PendingSuccessor`; admission functions in `link`. |
| `5bb7fec` | Withdrawal of sessions and links of a retired key. |
| `df3c79c` | The duplicate rule takes credential standing first. |
| `df435b5` | Withdrawal checks reduced; the check before a wait came back in `3f8a28e`. |
| `123c0c5` | Mutation faults K2 to K4, CR1 to CR15; P1 to P3, P12, L2, H14 to H16, Q26 follow the code. |
| `7ea3f60` | The documents of the redesign. |
| `ffa6513` | The per-contact endpoint claim corrected. |
| `836cccc` | How the specification and the code relate. |
| `799a6a8` | An imported card takes the place of a presented pending one. |
| `e015156` | Fuzz target `credential_sequence`; withdrawal in `session_frames`; seeds. |
| `13ef7bf` | A presented card does not displace an imported one; a second statement at the successor's epoch is a conflict. |
| `b077017` | The outbound request capability from the record. |
| `3f8a28e` | Admission before message 3; sticky withdrawal; the end of a link. |
| `d7691ab` | Crossing dials of two rotations. |
| `6ea04ac` | The documents of the fixes of section 8.3. |
| `b1ce3a6` | Mutation faults CR16 to CR21; H13 to H16 and Q30 follow the code. |

The commit that adds this table also brings `STATUS.md` and
`mutation/README.md` up to date.

## 9. Integration hardening

On 2026-10-02 two further static reviews of `phase-3-system-tor` found no
bypass of Noise XK or of F1, F2, F3 and F5, and reported seven issues in
how authentication, credential standing, identity disclosure, session
lifetime, budgets and socket I/O fit together. Section 9.1 records the
check of each against `c7faad2` before anything was changed, 9.2 what was
decided and changed, 9.3 the two review passes of the result, and 9.4 the
commits.

### 9.1 Verification of the reported issues

A. Message 3 after a withdrawal. Confirmed. `link::dial`
   (`crates/monolith-core/src/link.rs`) admits the responder after
   message 2, keeps the `Withdrawal`, and then writes message 3 with
   `write_all` without looking at the withdrawal again, neither before
   the write starts nor while it is pending. A key retired by another
   task after the admission still receives the local identity and card.
   On a single-threaded runtime nothing runs between the admission and
   the start of the write, but the write may suspend; on a multi-threaded
   one the window is open throughout. No test covers it: the race test
   retires the key before the admission.

B. A frame never has to complete. Confirmed. `Link::receive` wraps each
   socket read in its own `timeout(IDLE_TIMEOUT, ..)`, so every byte
   restarts the 240 seconds; a peer that sends one byte of a frame every
   239 seconds holds the link forever. `FRAME_READ_TIMEOUT` (60 seconds,
   "from length prefix to last byte of a frame") is declared in
   `limits.rs` and `RESOURCE_LIMITS.md` section 4 and used nowhere. The
   session age is checked in `AuthenticatedSession::receive` only when a
   frame is complete, so a partial frame also outlives the age limit.
   `UNKNOWN_FIRST_MESSAGE_TIMEOUT` and `UNKNOWN_SESSION_TIMEOUT`, which
   `PROTOCOL.md` sections 6.4, 7 and 12.1 make normative, are declared
   and never applied: an authenticated stranger holds one of the
   `MAX_UNKNOWN_SESSIONS` slots for as long as it keeps the stream open.
   No test covers any of this.

C. A failed read leaves the session usable. Confirmed. In
   `Link::receive` a read timeout returns `LinkError::TimedOut` and a read
   error `LinkError::Stream` without `stream_closed`; only end of stream
   closes the session. A later `receive` reads on, and a frame that
   arrives late is decoded and delivered. Errors of
   `AuthenticatedSession::receive` and of `Link::send` do end the
   session. A link whose session ended also keeps reading: the session
   drops input in `Closed` without an error, so `receive` waits on the
   stream again instead of failing.

D. Outbound sessions skip the stranger budget. Confirmed. `dial` returns
   every session with `unknown_slot: None`. If the record became none,
   declined or blocked during the dial, the session exists with a
   standing that is not a contact's and no slot of
   `MAX_UNKNOWN_SESSIONS`; message 3 has gone to that peer as well.

E. Fuzz targets compare accepted messages with the transcript. Confirmed
   as an incorrect assertion, not reachable in practice.
   `handshake_responder` asserts that an accepted message 1 equals the
   fixed one, with the comment that nobody else can make one; in fact
   anyone who knows the responder's public card can, with an ephemeral
   key of its own. It also asserts that an accepted message 3 equals the
   fixed one, and `handshake_initiator` the same for message 2 and 3. A
   different valid peer would make the target report a false failure. A
   random fuzzer does not find such inputs, since they need X25519 and
   the AEAD, so the targets have not failed; the invariants are still
   wrong. The known-answer vectors in `tests::vectors` are a separate
   test and are right.

F. The message 3 gate uses `Standing::StaleCard`. Confirmed in both
   directions. `StaleCard` stands for three causes: a card older than the
   active one that states the active key, a card with the retired key or
   older than the authorized successor, and a card that contradicts the
   active one or the successor at the same epoch. The first is the
   contact itself with an old card (a dial of a card whose endpoint
   change is not yet confirmed, or a peer restored from a backup), yet the
   dial withholds message 3 and the inbound session is not a contact's.
   In the other direction the gate lets message 3 go to a peer whose
   record became none, declined or blocked during the dial.

G. `PROTOCOL.md` contradicts itself. Confirmed. Section 4.4, check 3,
   says that message 3 is not sent to a key that is retired or pending
   and that no session is made; section 6.2 still says that for an
   initiator "the session is not a contact session", which presumes one.
   Section 12.1 lists the outcome for a contact with a stale card from
   the responder's side only.

None of the claims was wrong. The fixes keep the wire format, Noise XK
and the card layout.

### 9.2 What was decided and changed

A, F. One predicate decides whether the local identity goes in message
   3: `Admission::may_learn_local_identity`, true exactly for the
   standing of a requested or accepted contact. It is decided on the
   proven transport key and the credentials of that moment, not on a card
   object. For that, an older card of the active key is now
   `CredentialChange::Superseded`: its holder holds the key that stands
   for the contact and is the contact, with the standing of the record,
   and the card is not taken. `StaleCard` keeps the other causes, now
   listed exactly on the type. `link::dial` writes message 3 only if the
   predicate holds, and through `write_unless_withdrawn`, which looks at
   the withdrawal before the write and before every step of it; a
   withdrawal ends the dial and the session. Bytes the stream accepted
   before cannot be called back; nothing follows them.

   The order of the side effects is the brief's option B: the outbound
   admission records what the card changes when message 2 is accepted,
   before message 3, and is not undone when the write fails. For a peer
   that gets message 3 only `Advanced` and `Promoted` can be recorded,
   and message 2 alone justifies both, since it proves the key of the
   card. Undoing them would let the holder of the proven key decide, by
   breaking the stream, whether the local state follows its own proof;
   recording them after the write would put a wait between the admission
   and the record, which section 8 removed. A pending card is held as
   well when the peer gets no message 3, as for an inbound handshake: it
   gives no standing until the user confirms it.

   The dial card stays the caller's (brief item 25). Whatever card is
   dialed, message 3 goes only to a key that the contact state of the
   moment selects; a contact handle that the store hands out is Phase 4
   work.

B. The deadlines. `AuthenticatedSession` keeps when the frame in
   progress began, when the last complete frame arrived, and whether the
   peer was heard. `deadline()` gives the earliest of: the frame,
   `FRAME_READ_TIMEOUT` from its first byte and moved by nothing; the
   idle limit, `IDLE_TIMEOUT` from the last complete frame; the age
   limit, which no longer waits for a complete frame;
   `UNKNOWN_SESSION_TIMEOUT` in `AuthenticatedUnknown`; and
   `UNKNOWN_FIRST_MESSAGE_TIMEOUT` for a peer that is not a contact and
   has sent nothing. `expire()` ends the session: the frame and idle
   limits silently, as a failed stream; the others with a Close.
   `Link::receive` waits for the deadline beside the read and the
   withdrawal; the timeout per read is gone.

C. One end. `Link::finish` is the only way a link ends: the session is
   over, the slot for strangers goes back, the stream is shut down within
   `FRAME_WRITE_TIMEOUT`. The end of the stream, a read or write error or
   timeout, a deadline, a withdrawal, a violation and `close` go through
   it, and every later call fails at once.

D. Outbound sessions. With A, a dial makes a session only for a
   contact's key. A record that became none, declined or blocked, a
   pending or retired key and a conflicting card give no message 3 and no
   session, so an outbound session never needs a slot for strangers and
   none is taken. Taking a slot on a downgrade, the brief's alternative,
   would admit a session that should not exist.

E. The handshake targets check invariants: the genuine messages are
   always accepted, and an accepted message is judged by what it proves,
   not compared with the stored transcript. Seeds from a third party and
   from another ephemeral key make different valid messages; with the
   old comparison put back, both targets fail on those seeds. The
   known-answer vectors stay in `tests::vectors`. `session_frames` gained
   partial frames and expiry.

G. `PROTOCOL.md` 4.4 check 3, 6.2 with a table of what each standing
   means for message 3, the session, the slot and the end, 11.4 and 12.1
   now say one thing. `RESOURCE_LIMITS.md` section 4 says how each
   deadline is measured and how it ends a session. S51 and S52 are new;
   S36 and S50 were corrected.

### 9.3 The two review passes

Two focused reviews read the code of 9.2, one for privacy and
authorization, one for resources and lifecycle. Neither found a way
around the message 3 gate or the deadlines as designed. Fixed:

- A stranger that had sent its first message kept its slot until the
  idle limit when the caller did not close the link: the session logic
  enters Closing and the Close was the caller's to write, and the
  unknown-session deadlines apply only in `AuthenticatedUnknown`. A Close
  the logic decides is now due at once in the session, and
  `Link::receive` writes it, and ends the link, before it returns the
  message. A Close from the peer ends the link the same way.
- A failed seal inside `Link::send` ended the session without ending
  the link. It is now ended through `finish`.
- A withdrawal did not end a send whose write was pending; the frame
  could still reach the holder of the withdrawn key. `Link::send` now
  races the write against the withdrawal, as message 3 does, and ends the
  link without a Close when it wins.
- A dial whose authorized successor was replaced during the dial, by a
  newer announcement, held the dialed key as pending. A proven key that
  is older than the announced successor is now stale, as `announce`
  already treated it, and nothing is recorded.
- The gate of message 3 and the slot of a stranger read the admission the
  function returned beside the session, which a faulty function could
  contradict. Both now read the standing of the session.
- A failed admission function dropped the `Withdrawal` without marking
  it ended. It is now marked.
- Deleting or blocking a record was not said to withdraw the sessions
  of the identity, though `PROTOCOL.md` relied on it. It is now part of
  11.4 and of what the contact store of Phase 4 has to do.
- The documents: the conflict at the successor's epoch in both tables of
  6.2, the first 48 bytes of message 3 as what a partial write gives
  away, a full stranger budget that closes the newcomer until Phase 4
  rather than the oldest silent stranger, and two misplaced list items.
- Test gaps: bytes of a frame that would move the idle limit, a pending
  key and an older key on a dial, a failed admission, and in
  `session_frames` a deadline later than its own account.
  `session_frames` now checks every deadline against that account.

A check of the fixes against the findings found each one closed and no
regression. It found four small points, fixed in the documents and one
test: `Expiry::Close` and `RESOURCE_LIMITS.md` section 4 did not name the
Close that is due; the slot of a stranger is held until its Close is
written, not only until its first message; a withdrawal that comes while
a send is under way ends the link without a Close even before the first
byte, which the documentation of `Withdrawal` now says; and the failed
admission of `answer` had no test.

Left as residual, not changed:

- `HandshakeInitiator::read_message_2` returns message 3 before any
  admission, so the gate holds in `link::dial` and not in the session
  crate (S51). Moving message 3 behind `OutboundPeer::admit` changes the
  API of the handshake, its tests and the fuzz target, and is noted for
  Phase 4.
- A pending card that is not newer than a successor announced later
  stays held, and the user can still confirm it, but a handshake that
  proves it now gives `Stale` instead of `Pending`. Both give the
  standing of a stranger. `import` still holds such a card as pending:
  what the user brings in is the user's decision.
- A withdrawal that `Link::send` sees just after its outer timeout fired
  is reported as `TimedOut`; the link ends the same way.

### 9.4 Commits

On `phase-3-system-tor`, after `8f52eca`, oldest first:

| Commit | Content |
| --- | --- |
| `ebf5d07` | An older card of the active key is the contact (`Superseded`). |
| `980aa62` | Message 3 only while the key may learn the local identity. |
| `5cd1cff` | Session deadlines for frames, silence, strangers and age. |
| `4edca4a` | Every failure ends a link; the link waits for the deadlines. |
| `bae9026` | Handshake targets by invariants; partial frames and expiry in `session_frames`. |
| `f766554` | The withdrawal at every step of the message 3 write. |
| `0147eb3`, `c94e8c8`, `1adc603`, `e075fee`, `7a3be8e` | The documents of 9.2. |
| `d7d53ee` | A paused link test that waits forever fails. |
| `30ad278` | A proven key older than the announced successor is stale. |
| `b19acaa`, `6e22d53` | A Close the session decided is due at once; the idle limit test. |
| `1e8ca52` | A link ends when its session ends or a pending send is withdrawn. |
| `49fb144`, `88da043` | Message 3 and the slot from the session; failed admissions end. |
| `7ddf4f2` | `session_frames` checks every deadline against its own account. |
| `acd473b`, `755a045`, `4324da7`, `232d1a2` | The documents of 9.3. |
| `a8126a8`, `46c5901` | The check of the fixes: a test and the documents. |

The mutation faults CR22 to CR38, with CR12, CR16, CR18 and CR23 moved
and Q26 following the code, come in the commit after the last one above.
A targeted run of the new, moved and affected faults, 74 of them, without
the `monolith-tor` tests, caught all but S15 and S24, which are expected
to survive. The commit that adds this table also brings `STATUS.md` and
`mutation/README.md` up to date.

## 10. Final hardening

On 2026-10-03 the owner accepted the integration hardening of section 9
and asked for the last low-priority findings of two further static
reviews to be checked and closed before the final verification. Section
10.1 records the check of each against `94f5762`, before anything was
changed.

### 10.1 Verification of the reported issues

1. Terminal link operations repeat the shutdown. Confirmed.
   `Link::send` and `Link::receive` (`crates/monolith-core/src/link.rs`)
   look at the withdrawal before they look at whether the session is
   over. On a link that was withdrawn and has ended, every later call
   goes through `end_withdrawn` again: `AuthenticatedSession::withdraw`
   returns no frame, and `finish` shuts the stream down once more, which
   can wait up to `FRAME_WRITE_TIMEOUT` each time. `close` after the end
   does the same once. A link that ended otherwise fails at once through
   `over`. No test uses a stream whose shutdown stays pending; the mock
   and duplex streams shut down at once.

2. Controlled corruption of the handshake is no longer asserted.
   Confirmed. In the odd mode of `handshake_initiator` and
   `handshake_responder` one byte of the genuine messages is XORed with a
   value that is not zero, and the module documentation says that the
   handshake must then fail. Since the transcript comparison was removed,
   the targets assert only that a genuine message is not refused; a
   corrupted message that was accepted would pass. Every byte of the
   three messages is authenticated: the ephemeral keys are hashed into
   the associated data, and the rest is under the AEAD.

3. `CRYPTOGRAPHY.md` and ADR 0002 contradict F4 as implemented.
   Confirmed. `CRYPTOGRAPHY.md` section 5.1, F4, and ADR 0002, F4, say
   that a card older than the active card does not open a contact
   session. Since `ebf5d07` an older card of the active key is
   `CredentialChange::Superseded`: its holder is the contact and the card
   is not taken (`PROTOCOL.md` sections 6.2 and 11.4).

4. An outbound conflict is not reported by `dial`. Partially confirmed.
   The admission function receives the result of `OutboundPeer::admit`,
   so a function that looks at it sees `CredentialChange::Conflict`. But
   when the gate of message 3 refuses the peer, `link::dial` drops the
   admission and returns `ProtocolError::IdentityMismatch`, the error of
   a failed handshake. A function that returns the result of
   `OutboundPeer::admit` unchanged, as the tests and the development
   command do, leaves no trace of the conflict. `answer` returns the
   admission with the session in every case.

The three accepted residuals of section 9.3 were checked as well:

- Message 3 is prepared in the session crate. `link::dial` is the only
  production caller of `HandshakeInitiator::read_message_2`; the
  development command of the CLI dials through it. The documentation of
  the session crate still lists the direct calls as the way to dial,
  without saying that message 3 may only be written after the gate of
  the core.
- Import and admission. Admission and announcement never make a key
  older than the announced successor active: they give `Stale`.
  `Credentials::import` holds such a card as pending, which gives no
  standing; it takes over only if the user then confirms exactly that
  card (`Credentials::confirm`). No path lowers the epoch of the active
  card: a promotion needs a card newer than the active one, and the
  property test and the fuzz target `credential_sequence` assert it.
- Withdrawal against write timeout. Either way `finish` runs, the
  session is over and holds no key, the stream is shut down, and every
  later call fails.
