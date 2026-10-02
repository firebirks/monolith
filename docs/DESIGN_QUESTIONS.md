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
| F-R2 to F-R6 | Open points of the session layer: no external review of the binding rules, one maintainer of the library, no revocation of transport keys, stale cards towards parties without a record, no binding to the onion address | ADR 0002 |
| P1 | Padding block size | PROTOCOL.md 17 |
| P2 | Bind the session to the dialed onion key | PROTOCOL.md 17 |
| P3 | Epoch after restoring a backup | PROTOCOL.md 17 |
| P4 | Invisible declines cause indefinite retries | PROTOCOL.md 17 |
| P5 | Keep profile text in version 1 | PROTOCOL.md 17 |
| P6 | Message ordering across reconnects | PROTOCOL.md 17 |
| P8 | A period in which two transport keys are answered | PROTOCOL.md 17 |
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
