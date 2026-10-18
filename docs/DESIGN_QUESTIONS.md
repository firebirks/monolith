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
| CR-2 | A recipient-bound card format for per-contact endpoints | DESIGN_QUESTIONS.md 8.2 |
| CR-3 | A bound on how far an epoch may jump in one card | DESIGN_QUESTIONS.md 8.2 |
| P9 | Whether the card in a ContactRequest is still needed | PROTOCOL.md 17 |
| MI-1, MI-4 | Several local identities: a target port per identity on Tails and Whonix, and the phase that offers several in the interface | DESIGN_QUESTIONS.md 7 |
| C1 | `MaxStreams` value and semantics | TOR_CONTROL_SURFACE.md 6 |
| C2 | Proof-of-work queue parameters | TOR_CONTROL_SURFACE.md 6 |
| C3 | Confirming reachability on Tails and Whonix without `HS_DESC` | TOR_CONTROL_SURFACE.md 6 |
| T1 to T5 | Tails: the experimental preconditions (sandbox, OnionShare's path, required profile, packaging), profile matching, AppArmor, namespaces, Debian packaging. T1 blocks the Tails adapter. | PLATFORM_TAILS.md 3.4, 7 |
| W1 to W7 | Whonix: profile test, Qubes addressing, the two-Workstation isolation test (blocks the Whonix adapter), upstreaming the profile, SocksPort choice, a supported per-source port opening, KVM network design | PLATFORM_WHONIX.md 9 |
| ST3, ST5, ST6 | Previous generation, passphrase policy, Argon2id defaults (measured on the development machine only; the four targets of STORAGE.md 3.3 remain) | STORAGE.md 9 |
| ST1 | A message store for long histories; the store of version 1 is decided (ADR 0008) | DESIGN_QUESTIONS.md 12.3 |
| - | Whether history may be kept less: a "history off" mode, automatic retention, deleting single messages | DESIGN_QUESTIONS.md 12.3 |
| - | The per-contact rates of RESOURCE_LIMITS.md section 6, not enforced yet | DESIGN_QUESTIONS.md 12.3 |
| - | Several messages in flight per contact | DESIGN_QUESTIONS.md 12.3 |
| - | The order of the phases after Phase 5 | DESIGN_QUESTIONS.md 12.3 |
| - | Closing the oldest unauthenticated stream for a new one when the handshake budget is full | RESOURCE_LIMITS.md 5, 12 |
| - | A test of the idle run of S32: no traffic besides Tor's own | TEST_PLAN.md 11 |
| - | Pinning the Tor version or the image digest of the private network test | tests/tor-network/README.md |
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
      number of local identities one process may hold. Decided in Phase
      4: section 11, P4-7.

MI-3. Whether ephemeral and persistent identities can be mixed in one
      process (`STORAGE.md` ST7). Decided in Phase 4: section 11, P4-6;
      not in version 1.

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
    confirms. When to switch (CR-1) is set with the contact store:
    section 11, P4-10.

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

### 10.2 What was decided and changed

1. A link ends once. `Link::finish` shuts the stream down the first time
   and returns at once afterwards, and `send` and `receive` look at
   whether the session is over before they look at the withdrawal. The
   first terminal event may wait `FRAME_WRITE_TIMEOUT` for the shutdown;
   every later call, `close` included, returns at once. Tested with a
   stream whose shutdown never completes.

2. The handshake targets assert again that a genuine message with one
   byte changed is refused, while a valid message of another ephemeral
   key may still be accepted and is then held to the invariants. The
   seeds hold both kinds. A mutant of the session crate that ignores a
   failed Noise read does not make a corrupted message accepted, because
   Noise fails at the next step; such mutants are caught by the unit
   tests (H5, H6, H9).

3. `CRYPTOGRAPHY.md` and ADR 0002 state F4 as implemented: an older card
   of the active key opens a contact session and is not taken; a card of
   another key that is older than the active card or the authorized
   successor, a contradicting card and the retired key do not.

4. A dial that the gate of message 3 refuses returns
   `LinkError::Refused` with the admission, so a conflict, a pending key
   or a stale card is seen by the local side whatever the admission
   function does with it. Nothing changes on the wire: no message 3, and
   the stream closes as after a failed handshake.

5. The three residuals stay as accepted: the documentation of the
   session crate says that message 3 leaves the process only through
   `link::dial`; the different treatment of a card older than an
   announced successor by import and admission is recorded for the
   contact store of Phase 4 (`ARCHITECTURE.md` section 1.2, `STATUS.md`
   section 7); and a withdrawal that coincides with a write timeout may
   be reported as `TimedOut`.

6. `README.md` and the status line of `ARCHITECTURE.md` describe the
   branch as it is: the Tor integration is implemented and waits for its
   final verification.

7. The private network test was brought to the Chutney of today, which
   takes the network file through `init --net-from-script-path` and
   writes the torrc itself, and was extended: the two nodes exchange
   "hello" and "hello back"; a service whose control connection went
   away is published again from the key held in memory and reached again
   under the same name (`tests/tor_network.rs` in the core, ignored
   unless the script sets the endpoints); a dial without SOCKS fails;
   Tor B going away while B waits is reported as the control connection
   lost; and every Monolith process runs under `strace`, which must show
   only loopback and Unix sockets. The development dial tries again
   while Tor cannot reach the service yet, since a descriptor needs a
   moment to reach the directories.

### 10.3 Commits

On `phase-3-system-tor`, after `6bd9074`, oldest first:

| Commit | Content |
| --- | --- |
| `94f5762` | `README.md` brought up to the Tor integration. |
| `1343b68` | Section 10.1, the verification of the reported issues. |
| `e00e64a` | A link ends once; later calls fail without waiting for the stream. |
| `5e64924` | The handshake targets assert again that a corrupted genuine message is refused. |
| `08d91af` | F4 in `CRYPTOGRAPHY.md` and ADR 0002: an older card of the active key. |
| `e5a4fdc` | A dial refused before message 3 returns its admission. |
| `36551ad`, `754a3c7` | The documents: the message 3 boundary, refused dials, a link that ends once, the architecture status, the card evaluation of Phase 4. |
| `dfd12ea` | The development dial answers "hello back" and tries again while the service is unreachable. |
| `d0f9d2a` | The real-Tor test of publishing again from the held key and of a dial without SOCKS. |
| `0496610` | The private network test on the current Chutney, every node traced. |
| `c095969` | Mutation faults CR39 and CR40; CR12 and CR13 follow the code. |

The commit that adds this table also brings `STATUS.md`, `fuzz/README.md`
and `docs/TEST_PLAN.md` up to date. It is the last commit before the
final verification.

### 10.4 Final verification and completion

The final verification ran on `2f27dcb99df0f86bb1c539ff69b2323a43d75a05`,
from a clone at that commit, with no file of the repository changed while
it ran. Results:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked`, Rust 1.95.0, Linux | 517 passed, 0 failed, 1 ignored (`tests/tor_network.rs`, run by the private network test) |
| Rust 1.85.1: `check --all-targets` and `test` | clean; 517 passed, 0 failed |
| Rust stable (1.99.0): `test` | 517 passed, 0 failed |
| `cargo deny check` | advisories, bans, licenses and sources ok |
| `cargo audit` | no advisory for the 71 crates |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 (tests only, through proptest) and 0.10.1 |
| Seeds of `monolith-protocol`, `monolith-session` and `monolith-tor` | consistent |
| Known-answer vectors of `PROTOCOL.md` 16.1 | unchanged, pass |
| Fuzz, 14 targets, 25 seconds each | no failure |
| Fuzz, 900 seconds each: `tor_control_reply`, `socks_reply`, `session_frames`, `credential_sequence`, `handshake_initiator`, `handshake_responder` | no failure; 2.5 million, 85.6 million, 84 thousand, 12.3 million, 170 thousand and 233 thousand executions |
| Mutation suite, 218 faults | 209 caught, the nine expected survivors (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29), none unexpected, none invalid; exit code 0 |
| `tests/network/fail-closed.sh` | pass, all four cases |
| `tests/tor-network/two-node.sh`, Tor 0.4.9.13, Chutney `ae3a33c` | pass: "hello" and "hello back" over the private network, a service published again from its key and reached under the same name, a dial without SOCKS refused, a lost control connection reported; every Monolith process traced with loopback and Unix sockets only and no DNS |

The owner declared Phase 3 complete on that commit on 2026-10-03, with
the residuals A, B and C of section 10.2 accepted and no known security
blocker, and had the branch pushed at that commit; the requirement of the
brief that nothing be pushed was superseded by that instruction. The
commit is tagged `phase-3-complete`. T3-7 is closed by the private
network test. Phase 4 has not started.

Notes kept for the record:

- `a11539a`, a lint commit, also contains the Phase 3 list of
  `mutation/faults.py`, which was staged by mistake. It was not
  rewritten.
- An earlier full mutation run was discarded because two runners shared
  the same copies (`a59895d` prevents that), and an intermediate run was
  stopped because the final run had to be on the final commit; a test
  binary left over from the discarded run had slowed it down (`ea8e64e`
  makes the runner end the test binaries of a fault on a timeout and when
  it is stopped).
- During the final verification the long fuzz runs, first started in
  parallel, and the mutation run, first started with three workers and a
  shared target directory, were stopped for memory and started again on
  the same commit, one fuzz run at a time and with two workers each in
  its own directory.
- The two security reviews of Phase 3 that came before the credential
  binding review fixed: SAFECOOKIE pinned to the configured cookie file
  and checked before use; `Endpoint` opaque and checked again by the
  connection code; tokio networking kept out of the CLI and unbounded
  channels banned; the dial and unknown-session budgets enforced; a
  listener error no longer ending the accept loop; a failed write marking
  the session stream closed; intermediate base64 copies of a key erased;
  the trust placed in the control endpoint documented (`THREAT_MODEL.md`
  adversary O).

## 11. Phase 4: the contact store

Phase 4 began on 2026-10-05 from `2f27dcb`, the commit of the final
verification of Phase 3, on the branch `phase-4-contact-store`. Its brief:
one durable, transactional and authoritative contact subsystem, one
answer to what a card means whatever path it took, per-contact
serialization, retirement of live sessions, message 3 behind the
authoritative admission, invitations and several identities through the
store, the budget for strangers, the publication supervisor, and the
rotation and dialing policy. No change of Noise XK or of the wire format.

### 11.1 What Phase 3 left, and what became of it

- Import and admission judged a card of another key that is newer than
  the active card and older than the announced successor differently:
  pending when imported, stale when proven. Resolved by 11.2, P4-1.
- `HandshakeInitiator::read_message_2` returned message 3 before any
  admission; only `link::dial` enforced the gate. Resolved by P4-2.
- A withdrawal that coincides with a write timeout may be reported as
  `TimedOut`. Kept, and tested in the store era: whichever is reported,
  the link is over and every later call fails at once
  (`link::tests`).
- What `ARCHITECTURE.md` section 1.2 said the store had to add:

  | Item | Where |
  | --- | --- |
  | one lock, or one transaction, per contact around lookup, admission and keeping the withdrawal | P4-3 |
  | withdrawal of the sessions of a retired key in the step of the promotion, and of an identity whose record is deleted or blocked | P4-5 |
  | the confirmation and import actions on top of `Credentials` | P4-1, P4-3 |
  | persisting the credentials atomically, before the result is used | P4-4, P4-6 |
  | forgetting the withdrawals of links that ended, also those `answer` refused after the admission | P4-5 |
  | the timing of a rotation and of the switch to the new key | P4-10 |
  | the dial card the user confirmed, apart from the credentials | P4-11 |
  | one evaluation of a card for the import and the admission | P4-1 |

### 11.2 Decisions

P4-1 One evaluation of a card. `Credentials::relation` classifies a card
    against the credentials the same way whatever its source: the same
    statement, an older card of the active key, a newer card of the
    active key, the authorized successor's key, a new key, a conflict,
    stale. `Credentials::evaluate` applies one table from relation and
    source to an outcome and the credentials after it; `apply` makes the
    change. `contact::decide` adds the kind of record and the standing of
    a session, and is the only interpreter of a card of a known identity:
    `PeerRecord::admit` and `import`, and every operation of the contact
    store, call it. The old entry points (`admit`, `announce`, `confirm`,
    `import`, `replace`) delegate to the table. Where Phase 3 judged one
    card two ways, the conservative reading was taken: a card of another
    key that is not newer than the authorized successor is stale from
    every source, so an import no longer holds it as pending; a pending
    card that a later announcement through the active key supersedes is
    dropped; an older card of the active key is `Superseded` from every
    source and changes nothing. A key the identity superseded through
    its active key cannot be confirmed into place. Tests: the exhaustive
    table of `credential::tests::table` against a model written from
    `PROTOCOL.md` section 11.4, every record kind in every context
    (`contact::tests`), fuzz target `credential_sequence`.

P4-2 Message 3 in the admission. `read_message_2` returns the
    authenticated responder and keeps the handshake state;
    `OutboundPeer::admit` makes message 3 only for a standing that may
    learn the local identity and returns it with the session, and for any
    other returns the admission alone, so no message 3 exists for a peer
    that may not have it. `link::dial` takes the local identity, not an
    admission function: the store admits under the lock of the contact,
    the dial waits until the state the admission depended on is durable,
    and writes message 3 unless the session was withdrawn meanwhile. The
    wire bytes and the known-answer vectors are unchanged. A source scan
    (`tests/structure.rs` in the core) checks that only the store admits
    an outbound peer and that `link::dial` writes the message 3 of a
    granted admission once.

P4-3 The contact store. One store per local identity; one slot per
    remote identity, with its own lock, holding its record (declined,
    blocked, requested or accepted), for a contact the credentials, the
    card confirmed for dialing, the verification mark, the alias and the
    rotation marks, and the sessions admitted as the contact's. Every
    operation on one identity takes its lock, works on memory and lets
    go: they happen in the order they took it. The map lock is taken
    before a slot's and never while one is held; an identity without a
    slot is decided on under the map lock. No lock is held across an
    `await`. Operations on different contacts do not wait for each other.
    Tests: concurrent operations give the results and the state of one
    of their serial orders (`tests/store.rs`), different contacts lose
    nothing, a block that races an admission leaves no contact session
    standing.

P4-4 Commit before use. A change is applied in memory under its lock and
    stamped with the next generation of the installation. Revocations
    act at once. Anything that grants (message 3, a returned link, the
    success of a user command) waits until its generation is durable;
    with a vault a waiter that finds no write under way starts a job on
    the blocking pool, which writes the whole installation, puts the vault
    back and publishes the outcome, however the waiter ends, and later
    waiters find their generation covered. The snapshot it writes is one
    cut through every store. A crash between a change and its write
    loses the change and everything that depended on it, none of which
    was used. A failed write marks the installation failed: every session
    is withdrawn and every later change and admission refused, until the
    process starts again.

P4-5 One rule for live sessions. After every change of a slot, each
    session admitted as the contact's whose card no longer stands for a
    requested or accepted contact is withdrawn in the same step, and the
    links that ended are forgotten. That covers a promotion that retires
    a key, a block, a deletion, a confirmation of a pending key, and any
    later transition. Sessions admitted with another standing stay on the
    path of a stranger. Sessions do not survive a restart, so a restart
    cannot bring back a privilege the store revoked.

P4-6 The vault. Implemented as `STORAGE.md` section 3 specifies, with the
    payload layout of section 3.4 now written down. Decided with it: the
    lock of ST2 is an exclusive `flock` through `rustix`, which also
    gives the owner and permission checks of ST4 without `unsafe`; ST7
    and MI-3, `StateMode` is per installation in version 1: one vault
    holds every identity of a persistent installation, and an ephemeral
    installation holds only ephemeral ones. `MemoryDir` models what a
    crash leaves after each step of a write. New dependencies: `argon2`
    0.6.0, `hkdf` 0.13.0, `rustix` 1.1.5, `unicode-normalization`
    0.1.25 (`DEPENDENCIES.md`).

P4-7 Budgets of several identities (MI-2). Per identity: 16 inbound
    handshakes, 4 strangers, 256 contact sessions. Per process: 32
    inbound handshakes, 512 contact sessions, 4 dials, 8 local
    identities. Provisional, like the other values of
    `RESOURCE_LIMITS.md` section 5.

P4-8 Strangers. When the budget for strangers of an identity is full, the
    oldest silent stranger is evicted with a Close and the newcomer takes
    its slot; a stranger that has spoken is never evicted; evicted links
    that have not ended are bounded by the same number, so at most twice
    the budget exist at any moment. A contact is never in this budget.
    Definitions in `crate::strangers`.

P4-9 The supervisor. One per identity: publishes the service from the key
    the identity holds, runs its accept loop, reports it unavailable when
    Tor loses it, and publishes it again under the same name, after the
    reconnect delays of `RESOURCE_LIMITS.md` section 7. An identity
    without a stored key is not published again (`Publication::NoKey`).
    Shutdown ends it at once.

P4-10 Rotation (CR-1). One rotation at a time. Beginning one makes the
    new key and the successor card at the next epoch and stores both
    before anything is announced; the identity goes on answering with the
    old key. The successor is announced in an EndpointUpdate on every
    confirmed session made with the old key with an accepted contact that
    was not sent it yet. The switch, from which the identity answers with
    the new key and dials with it every contact that was sent the
    successor, is due once every accepted contact was sent it, or when
    the user says so. The rotation ends, and the old key is dropped, once
    every accepted contact confirmed a session made with the new key, or
    when the user says so. Until then contacts that were not sent the
    successor are dialed with the old key, which announces it to them.
    Each step reaches peers only once it is durable: the successor is
    announced, shown as the successor card, and the new key answers,
    dials and is handed out, from the generation that wrote the step
    (section 11.4). Each rotation has a
    number of its own, and what a contact did (it was sent the successor,
    it confirmed the new key) is recorded for that number: an
    announcement is completed with what it was made for, its rotation,
    its contact (that very record, not another one made later for the
    same identity) and its session, and a completion that comes after
    any of them changed counts for nothing. A confirmation counts for
    the contact its session was admitted for, read and marked in one
    step, and not for a record made again for the identity meanwhile.
    The switch and the end read their condition with the keys held,
    against the rotation they change, and a snapshot stores the rotation
    with the progress made for it and nothing else.

P4-11 Dialing. The cards dialed for a contact are its authorized
    successor, if it states the endpoints the user confirmed, and then
    the active key at those endpoints: the active card if it states them,
    the confirmed card if it states the active key. A pending successor
    and the retired key are never dialed, and a promotion that changed
    the endpoints leaves the contact undialed until the user confirms the
    new card. Importing a card that changes the active card, and
    confirming a pending card, confirm it for dialing. `link::dial`
    dials only a card the plan names when the dial starts, before a
    stream is opened: a card of an identity that is not a contact is
    refused (`LinkError::Refused`), and any other card the plan does not
    name fails with `LinkError::NoEndpoint`. A card that differs from a
    planned one only in its invitation capability states the same and
    is dialed. The plan a caller gets holds the local card, not the
    party.

P4-12 Invitations. `MAX_ACTIVE_INVITATIONS` stays 16. A capability is
    durable before the card that carries it is returned; revocation is
    durable; revoking and discarding removes exactly the pending requests
    that capability admitted. Version 1 has no expiry and no use counter
    (`PROTOCOL.md` section 12.2), so redemption is a request that enters
    the queue, which lives in memory, and acceptance is the durable
    creation of a contact.

P4-13 Records. An import of a blocked identity is refused until the user
    unblocks it; an import of a declined identity makes it a requested
    contact and removes it from the declined list. A confirmation is for
    a contact only.

P4-14 Several identities. `Installation::restore_identity` brings in an
    identity from its key material with the checks of S39, which also run
    when a vault is opened. The installation owns its identities: an
    identity whose installation is gone refuses every change and admits
    nobody. Deleting an identity closes it before it leaves the
    installation: it refuses every change and admission, holds no Onion
    Service key, and its accept loop and supervisor end and remove its
    service (section 11.4).

P4-15 Testing aids. `dev-node`, a node over a persistent vault driven by
    commands on standard input, is what the private network test drives;
    it is not the product's interface. `Budgets::with_limits` lowers the
    process budgets.

### 11.3 Not part of Phase 4

The brief of Phase 4 is the contact store, persistence, lifecycle and
credential convergence. The rest of the "contacts and queue" line of
`ARCHITECTURE.md` section 12 is not in it and was listed as work before
Phase 5 in the status file of Phase 4: the session manager that runs the
duplicate rule and the per-contact rates, the reconnect scheduler, the
outbound queue and the message store (ST1), and the command and event
interface of section 4 for a front end. The review of `MaxStreams` (C1,
T3-6) and the KDF benchmark on the four targets of `STORAGE.md` section
3.3 are also still open.

### 11.4 The review of the Phase 4 code

An independent review of the Phase 4 code, before its final
verification, found no path that gives a contact session or message 3 to
a key or a record that does not stand. It found the following, each
verified and fixed with a test and a mutation fault:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| A waiter for durability that was cancelled (a deadline, an aborted accept loop) dropped the vault and the lock of its directory, and the installation was not marked failed; later waits failed while changes were still made in memory. | A job on the blocking pool takes the vault, writes, puts it back and publishes the outcome itself, however the waiter ends. | `d5f7721` | `persist::tests`, CS45 |
| A waiter could miss an outcome and sleep until an unrelated write. | The same job; a wait looks at the outcome before the state, and the vault is back before the outcome is published. | `d5f7721` | `persist::tests::many_waiters_on_many_threads_all_finish` |
| A deleted identity went on admitting peers, writing message 3 and answering at its service, and its commits succeeded though no write held it. | Deletion closes the identity before it leaves the installation: no operation, no admission, no wait for durability, no Onion Service key; its accept loop and supervisor end and remove the service. Identity-level changes are refused before they are made. | `091c922`, `192b9f6` | `tests/identities.rs`, `tests/supervisor.rs`, CS46 to CS48 |
| The successor of a rotation could be announced, and after the switch the new key used, before the step was durable; a crash then left a contact with a key the vault did not hold, and the next rotation became a conflict at that contact. | Each step carries the generation that makes it durable and reaches peers only from then on (P4-10). | `192b9f6` | `tests/credentials.rs`, CS49, CS50 |
| A snapshot of the store read the slots one at a time and could see more declined identities or contacts than their bounds; the write then failed the installation. | The snapshot holds the lock of the map while it reads every slot; declining holds it across its two slots. | `5d13a57` | `contacts::tests`, CS51 |
| A dial or an answer cancelled while its admission was made durable left the session tracked by the store. | The withdrawal ends if that wait is cancelled. | `0088cf4` | `tests/credentials.rs`, CS53 |
| The parent of a newly created vault directory was not flushed. | It is, on every open that may create the directory, so a retry after a failed flush flushes it too. | `42de59e`, `2fa98e5` | none: a power loss is not modelled |
| The encoder left copies of what it encoded, keys of a vault record among them, in buffers it freed when it grew. | Growth moves the bytes and erases the old buffer; a writer erases what it holds when dropped. Best effort, as `CRYPTOGRAPHY.md` section 8 says. | `8875647` | `codec::tests`; not observable without `unsafe` |

A second review, of these fixes, found two paths they had left open and
two faults that did not work, all fixed:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| Ending a rotation made the new key the only one at once, also while the write of the beginning or of the switch was pending, so a card the vault could lose was handed out. | The end is due only after a switch that is durable, even when forced; a beginning stamps only the rotation it made. | `e430064` | `tests/credentials.rs`, CS54 |
| The withdrawal that makes a failed installation fail closed ran only in a task waiting for the failed write; when every such task was cancelled, sessions went on. | The write job runs the withdrawal itself when it fails. | `6d26193` | `tests/credentials.rs`, CS55 |
| CS49 and CS50 did not compile. | Rewritten. | `c42c698` | |
| CS52, a decline that lets go of the map between its two slots, was caught only some of the time. | Dropped: the window is a few instructions, and a fault caught at random gates nothing. The snapshot side of the same cut is CS51. | `c42c698` | |

The cancellation test of `0088cf4` now cancels the dial only once the
admission waits for the write (`20d3f35`).

The baseline of the mutation runner then found one more, under the load
of a whole test binary: the write job held the vault's slot, and since
`6d26193` the installation, until it ended, a moment after it had woken
the waiter, so an installation closed and opened again at once could
find the lock of its directory held. The job now holds both weakly and
lets go of everything before it publishes a durable outcome (`9c5ce64`).
That holds by construction; `tests/store.rs`
(`closing_an_installation_lets_go_of_its_directory_at_once`) exercises
it, and showed the old code failing in one run of five of the store
tests and the new one in none.

### 11.5 The closure of the review of the Phase 4 code

A further review of the Phase 4 code, at `e29d7f2`, reported thirteen
findings. Each was reproduced with a test that failed on the code it was
reported against, fixed, and given a mutation fault where a fault can
show it; all thirteen were real:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| The marks of a rotation's progress were booleans read apart from the keys: a snapshot taken while one rotation ended and the next began stored the new rotation with the progress of the old one. | Each rotation has a number; progress is recorded for a number, checked against the rotation in progress with the keys held, and the snapshot of an identity is one cut under the keys lock. | `abafe7e` | `identity::tests` (a hook between the reads), `contacts::tests`, `tests/credentials.rs`, CS77 to CS80 |
| `mark_announced` did not say which rotation it completed: an announcement made in one rotation and recorded after it ended counted for the next. | The completion carries the rotation; a stale one records nothing and returns false. | `abafe7e` | `tests/credentials.rs` (`the_progress_of_one_rotation_never_counts_for_the_next`), CS77 |
| Message 3 and frames were written by an aggregate write that the withdrawal was looked at around, not inside: a withdrawal between two writes let the rest of the bytes go out. | The link writes in its own loop and looks at the withdrawal before every write call. | `fdd6378` | `link::tests` (a stream that withdraws between two bytes), CS73 |
| A send dropped after it sealed its frame left a link that could be used again, with a frame cut short or a counter that had moved on. | A flag marks a write under way; the next call ends the link without a Close. A receive dropped as it ended the link keeps its message for the next receive, which ends the link first. | `7025aa1`, `cf5feba`, `890b93c`, `7480cb5` | `link::tests`, CS74, CS83, CS91 |
| What an admission recorded when no session followed (a refused dial, a dial or an answer without a slot) was not made durable and was lost by a restart. | It is made durable before the refusal is returned; a write that finds changes after its snapshot writes again before its outcome is out. | `6431341`, `fc239e2`, `3649b11`, `a849abc` | `tests/credentials.rs` (four restart tests), `persist::tests`, CS68 to CS72, CS88, CS89 |
| A deleted identity still signed cards, listed capabilities, and handed out a party to dial or answer with. | Every getter of a secret, a party or a signed card fails after the deletion; the identity's copies of its secret bytes are erased. | `d7c8f95` | `tests/identities.rs`, `identity::tests`, CS60 to CS62 |
| A publication under way was not ended by the deletion of its identity, and its service was reported available. | The wait for Tor ends on deletion and drops the publication; a service returned for a deleted identity is removed and not reported. | `493dcb7` | `tests/supervisor.rs`, CS75, CS76 |
| An invitation was listed and its card could be had before the capability was durable. | A capability is listed, carried and admits requests only once durable. | `2230ad9` | `tests/invitations.rs`, `requests::tests`, CS64, CS65 |
| Answering a request took it from the queue before the record changed: a refused answer (a full list) lost it, and a block could slip between the reading of a record and the queuing of a request. | The queue is locked while the record is read or changed. | `2bf7cad` | `tests/invitations.rs`, `identity::tests` (a hook), CS66, CS67 |
| `restore_identity` accepted secrets the vault refuses (all zero), which made the vault fail to open; a stored rotation to a clamping-equivalent key was accepted. | One rule for the secrets; the new key must be another public key. | `646cd05` | `tests/identities.rs`, CS57 to CS59 |
| The stored identity and rotation printed their secrets in `Debug`. | `Debug` written out, with the secrets redacted. | `854fcf7` | `record::tests`, CS56 |
| `apply` took a session of any identity. | The store that admits a session binds it; another identity refuses it. | `8e76687` | T-MI-10, CS63 |
| Buffers that held a capability or message content were not erased. | The signed bytes of a card, the copies of a card in a record and in its text form, the plaintext of every frame, the CLI passphrase and the passphrase normalization. Not observable without `unsafe`; `CRYPTOGRAPHY.md` section 8 lists what remains. | `f4d1a19` | none |

A second review of the whole Phase 4 code, and two reviews of its own
fixes, found more, all fixed:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| A dial waiting for its slot when its identity was deleted opened a stream; a session that outlived its identity, or a request considered as the identity was deleted, queued a request. | The deletion is looked at after the wait and with the queue locked. | `4c4f53d`, `90b9cbe` | `tests/identities.rs`, `identity::tests`, CS81, CS82, CS90 |
| A write job started a second job after its outcome was out, so an installation closed at once could find its directory locked, and on a runtime shutting down the installation was marked failed. Its first replacement capped the writes of a job, left changes unwritten past the cap, hid an earlier success behind a later failure, woke waiters before the sessions were withdrawn, and let a wait start a write after a failure. | The job writes again before its outcome: on while nobody waits, and a bounded number of times while a waiter is left, which then starts the next write itself; a later failure keeps what an earlier write made durable; a failure is recorded before the vault is back, and the sessions are withdrawn before any waiter wakes; the job holds the installation weakly. | `3649b11`, `a849abc`, `732f936` | `persist::tests`, CS70, CS88, CS89, CS92 to CS94 |
| An invitation card could state a key the identity no longer answered with. | Signed once the capability is durable. | `3d4aa84` | `tests/invitations.rs`, CS84 |
| The successor card and a new identity were shown before they were durable. | Shown only once durable. | `fedbf5a`, `1e343a6` | `tests/credentials.rs`, `tests/identities.rs`, CS85, CS86 |
| The rotation numbers of two identities were the same. | Each identity starts at random. | `ea8ed63` | `tests/identities.rs`, CS87 |
| An admission that failed after it changed a record would not stamp the change. No path reaches it. | It is stamped and the sessions are reconciled. | `3289cdc` | none |

The mutation suite: CS28, until then expected to survive, is caught by a
hook between taking a stranger's first message and delivering it; CS52
is back, caught by a hook between the two slots of a decline, and CS51
by a hook that checks, after every slot a snapshot reads, that it still
holds the lock of the map: deterministic in any order of the slots. Faults that the fixes had
moved were retargeted (`b06758c` and with their fixes). The structure
test read each source only up to its first `#[cfg(test)]`, indented
ones too, and so skipped most of `session.rs`; it reads up to the first
top-level test item now (`2da1a2c`).

A review of `642a5ed` from outside the project reported three findings
and two weaknesses of the mutation suite. Behind the findings is one
pattern: a logical name taken for an object instance (the same local
identity in another lifecycle, the same remote identity in another
record, the same counter in another identity, the same rotation context
in another rotation). Each was reproduced with a test that failed on the
code before it, and fixed; the search for the same pattern found more:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| Handles kept after the last `Installation` handle was dropped still had authority: party, Onion Service secret, invitation cards, dial plans, changes in memory; kept links still sent. | Dropping the installation closes its identities as a deletion does; a write under way ends on its own. | `cc8650f` | `tests/identities.rs`, `tests/supervisor.rs`, CS95 |
| `InvitationId` was a bare counter: an id of one identity worked at another. | Ids carry the instance number of the identity, drawn at random when the instance is made; so do rotation ids. | `b3a7cb5` | T-MI-11, CS96, CS87 |
| A late announcement marked a contact made again for the same identity. | `mark_announced` takes the `Announcement`, bound to its rotation, its contact record and its session. | `8c0aff1` | `tests/credentials.rs`, CS97, CS98, CS77 |
| The test of CS51 missed the fault in some orders of the map. | It checks at every read that the snapshot holds the lock of the map. | `f39f906` | `contacts::tests`, CS51 |
| CS47 and CS48 survived the whole core suite: other guards held. | Retargeted at the state they name. | `e6b8e18` | `tests/identities.rs`, CS47, CS48 |
| A request was answered by its sender: an answer for one request accepted a later one. | Requests have handles (`RequestId`). | `1314b9d` | `tests/invitations.rs`, CS99, CS67 |
| `delete_identity` deleted whatever identity held the keys it named. | It takes the instance. | `fc07b65` | `tests/identities.rs`, CS100 |
| A forced switch or end decided for one rotation hit the next. | Both take the `RotationId`. | `c73482e` | `tests/credentials.rs`, CS101, CS102 |
| `import` and `block` took out a request queued after them. | The queue is locked across the change. | `91a3415`, `780c3dc` | `identity::tests`, CS103, CS104 |
| A card was returned for a capability revoked as it was created. | The card is made from the active set after the commit. | `3a6a823` | `identity::tests`, CS105 |
| A `DialPlan` or the answering party handed out a party with its key. | Parties stay in the core; `DialPlan::local_card` instead. | `4c8e14b` | `tests/structure.rs`, CS107 |
| A closed identity showed the card of a key it no longer answered with. | It keeps the party it answered with. | `85d6a33` | `tests/identities.rs`, CS106 |
| A card of someone no longer a contact was dialed, up to message 2. | `dial` refuses before a stream. | `9f91b1f` | `tests/credentials.rs`, CS108 |
| A contact request was made with the card of the moment, refused while switched. | It is made with the card and capability of its session. | `069ad2c`, `ffb81cd` | `tests/credentials.rs`, CS109 |
| The dev node answered a request by sender, took ambiguous prefixes, and kept sessions of strangers as a peer's. | Fixed in the dev node. | `c8322e9` | the private Tor network test |

Two reviews of these fixes, one for the pattern, lock order and the
store, one for the dev node, the durability and the tests, found:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| A confirmation of the new key was checked against its session and marked in two steps; a contact deleted and made again in between got the mark. | One step, as for an announcement. | `b3767ba` | `identity::tests` (a hook between the steps), CS110, CS97, CS98 |
| `dial` dialed any card it was given: a pending successor, a retired key, endpoints the user did not confirm. | Only a card the dial plan names; the admission tests change the record while the dial waits for message 2. | `9d368dc` | `tests/credentials.rs`, CS111, CS112 |
| `apply` took the actions of a message from its caller, and a session apart from the message. | `Link::receive` returns an `Arrived`, made only there and bound to its session; `apply` takes that alone. | `2b22dd1` | `tests/structure.rs`, CS113 |
| A wait read what was durable before the failure, so it could report a failure for a generation its failing job had written. | It reads the failure first, and looks again when its write cannot start. | `fda2a25` | `persist::tests` (two hooks), CS114, CS115 |
| The module documentation of `persist` promised that every change is written, also after the installation closed. | Corrected: what was applied after the last snapshot is lost as in a crash, and nothing used it. | `69a11b2` | none |
| The dev node kept the entries of inbound sessions that `serve` aborted, dropped unsent texts silently, and printed the peer's acceptance as the user's. | A guard takes the entry out; unsent texts are reported; `peer-accepted`. | `8ea3ca9`, `1646bb0` | the private Tor network test |
| Two tests hung since `9f91b1f`, which refuses a dial before a stream: they waited for a stream on the other side. A failing test could hang elsewhere too. | The tests expect no stream; the dial helper ends when no stream comes; the tests run under deadlines. | `5d8caf2`, `d17ac9d`, `97eb6f0`, `eadc925`, `64e8354` | the suite itself |

A third pair of reviews, of everything above, found:

| Finding | Fix | Commit | Test, fault |
| --- | --- | --- | --- |
| An acceptance or a new card was checked against the withdrawal and the card of its session, not against the record that tracks it: a message kept from a link that had ended changed a contact deleted and made again since. | Every path asks the record whether it tracks the session. | `049de25` | `identity::tests`, CS116, CS117, CS97, CS110 |
| An `Arrived` could be applied again: a request applied after its answer and the deletion of its sender was queued again. | The first application marks it; a second fails (`StoreError::AlreadyApplied`). | `b4ab8b4` | `tests/invitations.rs`, CS118 |
| `mark_announced` took the caller's word that the successor was sent. | `Link::announce` writes it on the session it was made for and returns the `Sent` that `mark_announced` takes. | `bbda16f` | `tests/credentials.rs`, `tests/structure.rs`, CS119, CS120 |
| A `DialPlan` kept by a caller held the party, with the transport key, past the closing. | It holds the local card; `link::dial` takes the party from the identity. | `eb6cbf4` | `tests/structure.rs`, CS107, CS108 |
| `onion_secret` read the closing before taking the keys. | It reads it with the keys held. | `c9b31ff` | CS48 |
| The dev node printed an invitation handle in a form with spaces, which the network test read as several words. | One word, the same when shown and when named. | `8b7d4b0` | the private Tor network test |
| The snapshot test ran its step before the snapshot read anything; tests of persistence could hang on a failed assertion while a write waited at a gate. | A hook between the reads of a snapshot; the gate opens on unwinding. | `ae46ffd`, `4577d19`, `d24ab14` | `identity::tests`, `persist::tests`, CS121 |
| A message or an announcement used after its link was dropped counted until the record forgot the link. | Refused once the link ended. A cancelled `apply` counts as done. | `d490292` | `identity::tests`, CS122 |
| Found by a test that failed under load: a job put the vault back before it published its outcome, and a wait that looked in between started a write that held the vault after its own wait had returned. | The slot records what the job wrote as it puts the vault back; a start finds nothing to write. | `6225b3d` | `persist::tests` (a hook between the two), CS123 |

Looked at and left as they are: `LocalIdentity::wait_durable` fails
for a change made durable while its identity was being deleted; a
deleted identity reports nothing as done. `dial_plan` reads the mark
that a contact was sent the successor before that mark is durable; the
mark records what the peer already has, and a crash loses only the
record, which a later session announces again. `link::answer` takes the
identity and the stream it is given: `serve` gives each stream of a
service to the identity of that service with a handshake permit (S43),
and a caller that bypasses `serve` is not prevented; binding a stream
to its service in its type would change the Tor adapter. A link
of a stranger is not withdrawn when its identity closes; before
confirmation it may send nothing but a Close, the identity applies
nothing from it, and it ends at its first message or at
`UNKNOWN_SESSION_TIMEOUT`. CS81 now removes both checks of the closing
on the path of a late session (`73b4273`); with one left, the other
held, so the check in `apply` is a second line that no fault shows on
its own.

### 11.6 Final verification and completion

The final verification ran on `970316fc76d0ea19c1c65239b16556253a2894c8`
in the three stages of section 5.2 of the status file of Phase 4
(`STATUS.md`, removed when Phase 4 was merged), with no file of that
commit changed while it ran: stages 1 and 2 and mutation lists 1 and 2
from 2026-10-08, lists 3 and 4 on 2026-10-11 on a second machine (a Linux
VM with 8 CPUs, 8 GB and two workers). The outputs are in
`verification/970316f/`, merged from the branch `verification-970316f`.
The final report is section 5.3 of the status file, kept in the history
at `d2089b5`. Results:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked`, Rust 1.95.0, 1.85.1 and stable (1.95.0 that day), Linux | 33 test binaries, 682 passed, 0 failed, 2 ignored (the KDF measurement, and the test that the private network test runs), on each |
| Rust 1.85.1: `check --all-targets` | clean |
| `cargo deny check` | advisories, bans, licenses and sources ok |
| `cargo audit` | no advisory for the 82 crates |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 (tests only, through proptest) and 0.10.1 |
| Known-answer vectors of `PROTOCOL.md` 16.1 | unchanged, pass |
| Fuzz, 16 targets, 25 seconds each | no failure |
| Fuzz, 600 seconds each: `credential_sequence`, `contact_store`, `vault_payload`, `session_frames`, `handshake_initiator`, `handshake_responder`, `tor_control_reply`, `socks_reply` | no failure |
| Mutation suite, 338 faults | 329 caught, the nine expected survivors (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29), none unexpected, none invalid; exit code 0 for each list; the 123 faults of Phase 4, CS1 to CS123, all caught |
| `tests/network/fail-closed.sh` (T-NET-1), syscall observation via strace | pass, all four steps |
| `tests/tor-network/two-node.sh`, Tor 0.4.9.13, Chutney `ae3a33c`, `RESTART_CYCLES=15` | pass, all 12 steps; 15 of 15 restarts of Tor B recovered within the window, in 1 to 20 seconds |
| Identities of `2f27dcb..970316f` | 171 commits, all with `firebirks <336470935+firebirks@users.noreply.github.com>` as author and committer, no trailers |

The owner decided the four open points of the report on 2026-10-11. The
finding about eviction concerned the handshake budget, not the budget
for strangers, which evicts the oldest silent stranger as P4-8 says; so
`RESOURCE_LIMITS.md` was corrected (`8e7b5aa`) and the code was not. S32
no longer credits T-NET-1 with the idle run, which has no test
(`f549842`); not a blocker. T-NET-1 observes system calls through strace
where `TEST_PLAN.md` section 11 describes a packet capture: accepted.
The Dockerfile of the network test does not pin Tor: accepted; the run
used Tor 0.4.9.13, and pinning is hardening of the test infrastructure.

The owner then declared Phase 4 complete on `970316f` on 2026-10-11. The
commits after it, up to the tag `phase-4-complete`, change documents
only. From then on the mutation faults CS1 to CS123 and the regression
tests of Phase 4 are a baseline to preserve. T3-7 is settled for Phase 4
by the private network test. Phase 5 has not started.

Notes kept for the record:

- Three faults reached the runner's timeout of 20 minutes: T6 (Phase 2,
  under the load of four workers; alone it was caught in 791 seconds by
  failing tests), Q1 (also alone: without the SOCKS negotiation the test
  `an_onion_connection_carries_the_literal_hostname_and_then_the_peer_bytes`
  waits for a greeting that never comes) and CS30 (as `mutation/README.md`
  expects). Each also makes named tests fail, so none is a survivor
  hidden by the timeout.
- Left outside Phase 4, besides section 11.3: closing the oldest
  unauthenticated stream for a new one, a test of the idle run, and
  pinning Tor in the network test (section 3).

## 12. Phase 5: the messaging runtime

Phase 5 began on 2026-10-11 from `phase-4-complete` (`3c013f5`; the
verified commit of Phase 4 is `970316f`), on the branch
`phase-5-messaging-runtime`. Its brief: a reliable, persistent messaging
runtime without a user interface. A message is queued durably before it
is sent, stored durably at the receiver before it is acknowledged, sent
again while its delivery is uncertain, taken in once, recorded delivered
only by its acknowledgement, and survives restarts of either side, of
Tor, and the loss of a session; one owner for the sessions of each
identity, which keeps one when both sides dial, dials only through Tor
and only when there is something to deliver, and stops at once when a
contact or an identity is gone; commands and events a later interface
can use without any of the authority of the core; every resource
bounded. No wire change, no change of Noise XK.

### 12.1 What Phase 4 left, and what became of it

- `DESIGN_QUESTIONS.md` 11.3: the session manager with the duplicate
  rule, the reconnect scheduler, the outbound queue and the message store
  (ST1), and the command and event interface. Done: P5-1 to P5-12 below.
  The per-contact rates are not (section 12.3).
- A contact record and a local identity had no durable name: a record
  deleted and made again, or an identity restored from its keys, could not
  be told from the old one after a restart. Durable identifiers (P5-4).

### 12.2 Decisions

P5-1 Delivery semantics (`adr/0007-delivery-semantics.md`): durable
    at-least-once transport and exactly-once local acceptance through
    deduplication. A message is `Queued` until the acknowledgement of its
    identifier, from a session of the contact record it was queued for,
    is durable; then `Delivered`, for good. The receiver stores a message
    and its identifier in one locked step and acknowledges it only once
    that is durable; a duplicate is acknowledged again and stored nothing.
    One message in flight per contact, first in, first out.

P5-2 No wire change. ChatMessage and MessageAck (`PROTOCOL.md` sections
    8.5 and 8.6) carry what the runtime needs; their texts now say that the
    acknowledgement follows durable storage and that the duplicate check
    is in the message store. Noise XK, the frames and the known-answer
    vectors are unchanged.

P5-3 The message store (`adr/0008-message-store.md`): a container of
    the vault's kind, `messages`, under a key HKDF derives from the vault
    key, rewritten whole and replaced atomically, at most 16 MiB, with a
    write job and generations of its own. SQLCipher was not taken:
    `rusqlite` needs Rust 1.88, above the minimum version 1.85.1, and it
    brings C code and OpenSSL into the process. ST1 is closed for version
    1; a store for long histories stays open.

P5-4 Durable identifiers. Vault payload version 2 gives every local
    identity an `IdentityId` and every contact record a `ContactId`, 16
    random bytes each, never changed or reused. Messages are bound to
    them. A vault of version 1 is still read; its records get identifiers
    when it is opened, after everything in it was checked, and it is
    written as version 2 before anything refers to them.

P5-5 A persistent installation always has a message store: acknowledged
    acceptance needs it. Messages are kept until the user deletes their
    conversation. This replaces "history off by default" of `STORAGE.md`
    section 1, which held queued messages in memory and so could not
    survive a restart; whether history may be kept less is open (section
    12.3). An ephemeral installation keeps everything in memory, messages
    included.

P5-6 The session manager (`adr/0009-session-manager.md`): one per local
    identity instance, keyed by `ContactId`. A session is registered when
    it is confirmed and stands for the record; the duplicate rule of
    `PROTOCOL.md` section 14 keeps one, with a Ping that probes an older
    preferred session; only the preferred session sends the queue; any
    registered session takes messages and acknowledgements in.

P5-7 No keepalive. A Ping is sent only to probe a duplicate session; a
    session with nothing queued or in flight for `SESSION_LINGER` is
    closed by the local side. `PROTOCOL.md` section 8.2 makes keepalive
    Pings optional; Pings from a peer are answered.

P5-8 Reconnecting: one scheduler per conversation with queued messages,
    a current record and no session; it dials through the Tor backend
    with the delays of `RESOURCE_LIMITS.md` section 7, the first attempt
    at once after a send and spread over `STARTUP_DIAL_SPREAD` at
    startup. A contact with nothing queued is not dialed.

P5-9 Lifecycle. Blocking or deleting a contact ends its record: its
    sessions are withdrawn (Phase 4), its scheduler ends, its queue stays
    stored, ended, and is never sent; a record made again starts empty. A
    new card or a rotation of the same contact keeps the record and its
    queue. Deleting an identity drops its messages, after the vault no
    longer holds it; an identity restored from its keys starts empty.

P5-10 `MESSAGE_ACK_TIMEOUT`, 60 seconds: a message whose acknowledgement
    does not come is uncertain; the session is closed and the message is
    sent again on a later session, after the reconnect delay.

P5-11 Events are a bounded broadcast of `MAX_UI_EVENTS`
    (`RESOURCE_LIMITS.md` section 8), never with message text; one that
    reports stored state is raised after that state is durable, the others
    report the runtime (ADR 0007); a consumer that falls behind is told
    how many it missed and reads the state again. Nothing waits for a slow
    consumer.

P5-12 The boundary of a front end: the plain values of `monolith_core::api`
    and the crate `monolith-runtime`, whose `Messenger` hands out no local
    identity, installation, link, session, key or capability. A front end
    that depends on it alone cannot reach them.

P5-13 What arrived on a link that ended counts for nothing, for a chat
    message and an acknowledgement as for a change of the contact store
    (CS122): it is not stored and not applied; the sender sends again.

P5-14 Limits (`RESOURCE_LIMITS.md` sections 4, 5 and 9): the queue per
    identity, the conversations per identity, the bytes per conversation,
    the messages and the size of the store, and the two timeouts above.
    `MAX_QUEUED_MESSAGE_BYTES` is lowered from 64 MiB to 8 MiB, half of
    the store.

P5-15 Test tooling: `dev-runtime` in the CLI, a node on the runtime for the
    private Tor network test, and `RuntimeOptions::acknowledgement_delay`,
    which can only delay an acknowledgement, to place a crash between
    storing and acknowledging. `dev-node` stays as it was for the steps of
    Phase 4.

### 12.3 Not part of Phase 5, and open points

- The order of the phases after Phase 5: `ARCHITECTURE.md` section 12
  numbered file transfer as Phase 5 and the GUI as Phase 8; the brief of
  Phase 5 names the GUI on the runtime as Phase 6. The table is changed
  for Phase 5 only; the owner's next brief sets the rest.
- Keeping less history: a "history off" mode, automatic retention,
  deleting single messages. Version 1 deletes whole conversations.
- The per-contact rates of `RESOURCE_LIMITS.md` section 6
  (`CHAT_MESSAGE_RATE` and the others) are not enforced: a contact is
  bounded by the limits of its conversation and of the store, not by a
  rate.
- A message store for long histories (an append-only log, or SQLCipher
  with a higher minimum Rust version).
- Several messages in flight per contact, for throughput on slow
  circuits.
- The queued messages of an ended conversation (a contact blocked or
  deleted) stay stored and count toward the limits of the queue of the
  identity until the user deletes that conversation. Whether they should
  be dropped, or counted apart, is a decision about what the user keeps.
- An identity whose deletion is cancelled after its vault commit, outside
  the runtime, keeps its messages in the store until the next opening,
  which drops them. Through the runtime a deletion is not cancelled.
- Commands for contact requests and invitations through the runtime: the
  core has them (Phase 4); the runtime offers adding, blocking and
  deleting a contact only. A front end of a later phase wraps the rest.
- What the runtime does not take in yet: Profile messages and file
  transfer are ignored when they arrive.
- From Phase 4, unchanged: the review of `MaxStreams` (C1), the KDF
  measurement on the four targets (ST6), a target port per identity on
  Tails and Whonix (MI-1), the phase that offers several identities in an
  interface (MI-4).

### 12.4 The internal review of the Phase 5 code

A source-first review of the code of commits `06dceb8` to `7aebaf4`,
against the list of the brief (a stale generation, a contact or an
identity made again, an acknowledgement or a queue that reaches another
owner, an effect outside before durability, cancellation after work that
cannot be undone). Each finding was reproduced by a test that failed
before its fix.

| Id | Finding | Fix | Regression |
| --- | --- | --- | --- |
| R5-1 | A message queued while a session of its record was open could be sent before its queueing was durable: the change woke the task of the session before the message store was written. A crash then left the peer with a message the sender did not keep. | `LocalIdentity::next_to_send` returns the head only once the message store holds it; the runtime sends only what it returned. | `tests/messages.rs` (offered only once durable), `tests/structure.rs`; MR31, MR32 |
| R5-2 | After a failed write of the message store, a session resent the head of its queue after every acknowledgement it could not record, without end, and schedulers went on dialing. | A failed message store ends every session and scheduler of the runtime; an acknowledgement that could not be recorded raises `StorageFailed`. | `tests/runtime.rs` (a failed message store ends every session and attempt); MR33, MR34 |
| R5-3 | Blocking or deleting a contact did not wake its scheduler: a delay ran out and a dial in progress went on, and its failure was reported as a deferred delivery of the ended record. | Closing the sessions of a record wakes whoever follows the sessions, and the scheduler cuts a delay or a dial short when its record ends. | `tests/runtime.rs` (blocking or deleting ends the attempt in progress); MR35, MR36 |
| R5-4 | Two candidates of the brief for faults had none: a message recorded delivered once it is handed to a session, and a message in flight reported delivered when its session ends. | Faults MR29 and MR30, and a test of a session that ends with a message in flight. | `tests/runtime.rs`; MR29, MR30 |
| R5-5 | A new function of the contact store repeated the text of fault CS51, which then matched twice. | The new code is worded otherwise; CS51 is unchanged. | the precheck of `mutation/run.py` |
| R5-6 | The comment of `SessionManager::register` said that `current` is asked under no lock of the manager; it is asked under it. The lock order (manager, then contact) holds everywhere. | The comment says what happens. | - |
| R5-7 | The duplicate check is per contact record: a contact made again can take in again a message its old record had taken in. | Kept, as S56 requires; written down in ADR 0007. | - |

### 12.5 The two independent reviews

Two reviews of the same tree (commit `7aebaf4` with the changes of
section 12.4), each made without the other's findings, against the list
of the brief. Every finding was checked against the code; the ones that
hold were reproduced by a test that failed before its fix, and each fix
has a mutation fault.

| Id | Finding (review) | Fix | Regression |
| --- | --- | --- | --- |
| R5-8 | A session that a service's accept loop ran was aborted with the loop when the service went away (Tor restarted) and stayed registered for good: no scheduler started for its record, a duplicate probe went to a task that was gone, and the front end saw a session (A, B) | The end of a session is handled when its task is dropped, however it ends | `tests/runtime.rs` (a session that ends with its service leaves no registration behind); MR38 |
| R5-9 | A session that ended for want of an acknowledgement always lasted `RECONNECT_RESET_AFTER`, so the delays started again each time: a peer that never acknowledges was dialed every minute or so for good (A) | Only a session that ended otherwise starts the delays again | `tests/runtime.rs` (growing delays); MR39 |
| R5-10 | A head that waited for the store to be written was sent even if the user deleted the conversation meanwhile (A, B) | `next_to_send` looks again after the wait and sends only what is still the head | `tests/messages.rs`; MR37 |
| R5-11 | A vault with identities and no message store got an empty store, silently, and a vault of Phase 4 put back beside a store with messages had them dropped (A, B) | The first opens, as a copy of the vault alone has to (Phase 4, T-MI-9), and says that the messages are not there (`message_store_missing`); the second is refused (`InstallationError::MessageStore`). Refusing the first was tried and broke T-MI-9 | `tests/messages.rs`, `tests/identities.rs` T-MI-9; MR40, MR41 |
| R5-12 | A crash while the message store was being made left a `messages.new` that stopped the installation from opening for good (B) | Where nothing can be lost the store is made again over what the creation left; for a vault of Phase 4 the store is made and read before the vault is written as version 2 | `tests/messages.rs` (a crash at every step of both); MR42 |
| R5-13 | A runtime dropped without a shutdown ran on for nobody: its tasks held it, services stayed published, keys in memory, the directory locked (B) | The last handle stops it as a shutdown does | `tests/runtime.rs`; MR43 |
| R5-14 | After a failed vault write the schedulers went on dialing (B), and `StorageFailed` was not raised on every path (A, B) | A failed vault or message store ends every session and scheduler, and is reported once | `tests/runtime.rs` (a failed vault ends every attempt); MR44 |
| R5-15 | A message or a received message of an identity being deleted could enter the state after its messages were removed, between the check of the identity and the change (A, B) | The check is made under the lock of the messages | unit test in `identity.rs` (with a hook); MR48 |
| R5-16 | A send its caller stopped waiting for left the message in memory without its event or a scheduler, and a vault that failed under a send left it there too; the same for the other commands (A, B) | Commands run as tasks of their own and complete; the record is made durable before the message enters the state | `tests/runtime.rs`; MR49 |
| R5-17 | Sections of identities that had to go could exceed what a payload holds (8) and fail the store (B); a conversation whose sequences ran out overwrote an entry (B) | The state refuses a ninth identity section and a full sequence | unit tests in `messages.rs`; MR45, MR46 |
| R5-18 | A change that changed nothing woke every session task (B); a scheduler looked up its record in O(contacts) on every wake (A) | Only a change wakes them; `LocalIdentity::is_current` reads one slot | unit test in `messages.rs`; MR47 |
| R5-19 | `Runtime::connect` dialed with a session in place (B); `dev-runtime send` took the text from the wrong place with repeated spaces (A, B) | Fixed | - |
| R5-20 | The test of a send its caller stops waiting for failed every time in one build and passed with traces, the profile of a lost wakeup; the owner made it a blocker of the freeze. Reproduced step by step: the tooling that mirrors the tree for the Linux build gave a file restored after a fault trial its older time back, so cargo ran the binary built with fault MR49, whose behaviour that was. Not a defect of the code. The wait for durability of Phase 4 (`persist::Store::wait`), which the messages share, marks the outcome seen before it looks at the state, so an outcome published in between ends the wait | The mirroring tool keeps the newer time of a file with the same bytes. The wait is covered by a deterministic test that publishes the outcome at each point between its check and its sleep, and must end in one poll. The same cause then showed once more: a test of a crash while the store is made failed with the store library still built with fault MR42; the mirror's builds were thrown away and everything built again. The mutation runner was not affected (its copies come from `git archive`, and it restores a file with a newer time), and now checks it on each copy before a run: a known fault is caught, and once it is restored the crate is built again and passes | `persist::tests`; MR50; the check of `mutation/run.py` |

Not changed, with the reason:

- Queries show the state in memory, which can be ahead of the store by a
  write under way (A). Written down in ADR 0007: if that write fails, the
  store fails closed and is reported.
- Blocking or deleting by the record names the remote identity of the
  record; a deletion and a new import of the same identity between the
  two steps of one command, on another thread, would act on the new
  record (A). No await separates them; the outcome is that of the other
  order.
- The queued messages of ended conversations count toward the limits of
  the identity (B): section 12.3.
- Two sessions of the same initiator registered in different orders at
  the two sides can close each other; only an explicit `connect` makes a
  second one, and the scheduler dials again (B).

### 12.6 A review of the fixes

A third review, of the fixes of sections 12.4 and 12.5 alone (commit
`35428bf`), found nothing critical or high. What it found, checked
against the code:

| Id | Finding | Fix | Regression |
| --- | --- | --- | --- |
| R5-21 | A send to an installation whose vault failed was refused as an identity that is gone, and a vault that failed with nothing running was never reported, or only once a scheduler woke | A failed vault is a failed store for a send; a task of the runtime follows the failures of the vault and of the message store and reports one at once | `tests/runtime.rs` (a failed vault is reported at once and refuses a send); MR51, MR53 |
| R5-22 | An identity deleted while a send waited for the vault made the send report a failed store, and raised `StorageFailed`, once for the process, though nothing failed | A wait that ended because the identity closed is a closed identity | `tests/runtime.rs` (an identity deleted while a send waits for the vault fails nothing); MR52 |
| R5-23 | An inbound session that ended for want of an acknowledgement still started the delays of the scheduler again | The end of each session is kept per record; only one that did not time out starts them again | `tests/runtime.rs` (a peer that dials in and never acknowledges does not reset the delays); MR54 |
| R5-24 | Commands ran outside the tasks of the runtime: a shutdown neither waited for them nor ended them | They are tasks of the runtime: a shutdown waits for them within its deadline, then ends them; a write of the vault under way ends on its own (Phase 4) | `tests/runtime.rs` (a shutdown waits for a command under way); MR55 |
| R5-25 | `SessionAvailable` and `SessionUnavailable` did not alternate: a session closed by the duplicate rule could report a record unavailable twice | They are raised only when whether a record has a session changes, decided under one lock | `tests/runtime.rs` (both sides at once: the events alternate); MR56 |
| R5-26 | The end of a session that was never registered woke every scheduler of the identity, and the list of conversations looked up every contact for each | Only a change wakes them; the list uses `is_current` | unit test in `sessions.rs`; MR57 |
| R5-27 | A vault of Phase 4 refused beside a store with messages gave the front end only "does not validate" | `OpenError::MessageStore` | - |

Not changed, with the reason: a session that waits for the message store
to write the head of its queue before it sends it reads nothing in the
meantime, also not a close or a Ping (R5-28). The wait is one write of the
store, a small file on local disk; a withdrawal still stops the send,
since the link refuses to write once withdrawn, and at-least-once makes a
message sent once more harmless. Waiting for the write inside the select
of the session would add a state for little.

Before the freeze the official runner caught the eleven faults of Phase 4
in `persist.rs`, then stopped before MR50: the unmodified run of its copy
did not end within the fixed bound of 20 minutes (R5-29). With Phase 5
that run takes about 16 minutes on this machine, and faults run beside
each other took up to 20. A run that only reached the bound was counted
caught, so a fault that survived could have been counted caught. The
bound is now measured from the unmodified run (`mutation/README.md`).

### 12.7 The final verification of 75ccf7c

The final verification of `75ccf7c` passed every step up to the
mutation lists, and the Phase 5 list caught 56 of its 57 faults. MR28
survived (R5-30), so `75ccf7c` could not close the phase; the lists of
Phases 4 to 1 were not run on it.

| Id | Finding | Fix | Regression |
| --- | --- | --- | --- |
| R5-30 | MR28 removes the check that a session has no message in flight before it sends the head of the queue. The message in flight stays the head until its acknowledgement is taken, and nothing behind it is sent before that is durable, so the fault cannot send the next message, as its entry said: it sends the message in flight again whenever the session wakes, and its acknowledgement is then awaited from the last send. Against PROTOCOL.md 8.6 (a message is sent again on the next session) and `MESSAGE_ACK_TIMEOUT`, a peer that never acknowledges keeps the session open as long as something wakes it within that time: a message queued behind, or any frame the peer sends. No test queued more while an acknowledgement stayed away and looked at when the session ended | A test: while the acknowledgement of a message does not come, two more are queued behind it, and the session must end `MESSAGE_ACK_TIMEOUT` after the one send. Any send in between, of that message or of another, starts the wait again and fails it. The entry of MR28 now says what the fault does. The fault was in the list since `7aebaf4` and had not run until then: the run on `7aebaf4` was stopped after MR12, and later trials covered the faults added or moved after it | `tests/runtime.rs` (a message in flight is sent once and waited for from then); MR28 |

MR25 and MR26 were caught by the timeout of the runner alone, and MR9 by
the timeout and `a_failed_attempt_is_not_repeated_at_once`. Each fault
turns a loop into one that never waits: MR25 and MR26 leave a deadline in
the past that the loop sleeps until and checks against a later one, MR9
dials again at once. With time paused the runtime is then never idle,
time does not move, and the tests that wait for the session to end wait
for ever; a bound in time inside a test would never fire either. This is
the case of CS30 in Phase 4.

### 12.8 Final verification and completion

The final verification ran on `91bf22cd60930e52f53229bde389e8f115927e64`
from 2026-10-12 16:41 to 2026-10-14 17:37, with no file of that commit
changed while it ran: the container stage (Debian 12, Rust 1.95.0,
1.85.1 and 1.99.0, Tor 0.4.9.13, Chutney `ae3a33c`) on an export of the
commit checked file by file, the WSL stage (Debian 13.4) and the mutation
lists, two workers, in a clone at the commit. The outputs are on the
branch `verification-91bf22c`, not on `main`. The final report is
section 6 of the status file of Phase 5 (`STATUS.md`, removed when Phase
5 was merged), kept in the history at `78881c1`. Results:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean |
| `cargo test --workspace --locked`, Rust 1.95.0, 1.85.1 and stable 1.99.0, Linux | 39 test binaries, 782 passed, 0 failed, 2 ignored (the KDF measurement, and the test that the private network test runs), on each |
| Rust 1.85.1: `check --all-targets` | clean |
| `cargo deny check` | advisories, bans, licenses and sources ok |
| `cargo audit` | no advisory for the 83 crates |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 (tests only, through proptest) and 0.10.1 |
| Known-answer vectors of `PROTOCOL.md` 16.1 | unchanged, pass |
| Fuzz, 18 targets, 25 seconds each | no failure |
| Fuzz, 600 seconds each: `message_store`, `message_sequence`, `vault_payload`, `contact_store` | no failure, no artifact |
| Mutation suite, 395 faults | 386 caught, the nine expected survivors (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29), none unexpected, none invalid; the 57 faults of Phase 5, MR1 to MR57, and the 123 of Phase 4, CS1 to CS123, all caught |
| `tests/network/fail-closed.sh` (T-NET-1), syscall observation via strace | pass |
| `tests/tor-network/two-node.sh`, Tor 0.4.9.13, Chutney `ae3a33c`, `RESTART_CYCLES=15` | pass, all 20 steps, in 2085 seconds; 15 of 15 restarts of Tor B recovered with one attempt |
| Identities of `phase-4-complete..91bf22c` | 21 commits, all with `firebirks <336470935+firebirks@users.noreply.github.com>` as author and committer, no trailers |

The mutation run was interrupted twice at the owner's request and once by
a restart of the machine, each time resumed on the same commit from the
faults that had not ended. Seven faults reached the runner's timeout:
CS30, MR25 and MR26, which turn a loop into one that never waits (section
12.7); MR9, Q1 and T6, which run again alone also made named tests fail;
and CS29, whose four supervisor tests were not reached because the wait
for publication in `Peer::start` of `tests/runtime.rs` lasts hours under
that fault. Run on their own with CS29 applied, those four tests fail as
on `970316f`.

A first final verification, of `75ccf7c`, had stopped at the Phase 5 list
when MR28 survived (R5-30); `07d6335` added its test and `91bf22c` was
frozen after it.

The owner accepted the report and its verdict on 2026-10-14 and declared
Phase 5 complete on `91bf22c`. The commits after it, up to the tag
`phase-5-complete`, change documents and comments only and do not change
the verified runtime: the report (`78881c1`); a correction from the
check of the report against the code (`65f1db5`): an event that reports
stored state follows its durability, while the events of the runtime,
`IdentityClosed` among them, report state that is not stored, and
`RefusedReason::Storage` is reserved and not raised; this record; and the
removal of the status file. From then on the mutation faults and the
regression tests of Phases 1 to 4, and MR1 to MR57, are a baseline to
preserve: none is weakened without an explicit design decision.

Left for later, besides section 12.3:

- Mutation tooling: a quick and deterministic kill of CS29, by bounding
  the wait for publication of `Peer::start` (and `until`) in
  `tests/runtime.rs` to minutes of test time.
- ADR 0009 names a card confirmed for dialing among the conditions of a
  scheduler; the runtime starts one without it, and a record without a
  dialable card is reported `DeliveryDeferred` with `NoEndpoint`, with
  the growing delays, and never dialed.
- `cargo doc` with warnings denied finds three intra-doc links in the
  module documentation of `persist.rs`, `messages.rs` and `sessions.rs`
  that do not resolve or name private items. Documentation is not built
  by the verification.
- Rust issue rust-lang/rust#164250 (unsound, no fix yet): Monolith has no
  type whose `Drop` relies on a `'static` bound of its parameter, the
  pattern it needs; building and testing again with the stable compiler
  that fixes it, once released, is a check to make.

## 13. Phase 6: the desktop client

Phase 6 began on 2026-10-14 from `phase-5-complete` (`3be1254`; the
verified commit of Phase 5 is `91bf22c`), on the branch
`phase-6-desktop-client`. Its brief: a desktop client in Rust with egui
and eframe that a person who does not know the CLI can install and use:
make or choose a local identity, see whether Tor is available, add
contacts, answer requests, make and revoke invitations, read and send
messages that show `Queued` and `Delivered` as the runtime says, use the
client while a peer or Tor is away, block and remove contacts, switch
between identities, and start again where they left off. The runtime of
Phase 5 stays the authority; the GUI holds none of its objects. It must
not look like a stock egui application. No wire change, no change of the
delivery semantics.

### 13.1 Decisions

P6-1 Toolkit (`adr/0010-gui-toolkit-egui.md`): egui and eframe 0.32.3,
    the newest release that builds with Rust 1.85.1; the `glow` renderer,
    X11, Wayland and AccessKit. eframe brings an URL opener that Monolith
    never lets act: every `OpenUrl` is removed from egui's output each
    frame.

P6-2 Crates and boundary (`adr/0011-ui-security-boundary.md`):
    `monolith-ui-model` (state, no egui), `monolith-ui-kit` (the design
    system), `monolith-ui` (screens) depend on no Monolith crate;
    `monolith-desktop`, the binary, is the only one that uses
    `monolith-runtime`, through `Messenger` and its plain values, and
    turns them into handles of the UI model.

P6-3 Design system (`adr/0012-design-system.md`): one theme of tokens, a
    light and a dark palette, Inter, JetBrains Mono, a Lucide icon font cut
    to what is used, and components with every interaction state; no stock
    egui widget on a screen.

P6-4 What the runtime adds for a front end, wrapping what the core already
    does: contact requests (list, accept, decline, block the sender),
    invitations (create, list, show the card again, revoke, revoke and
    discard their requests), the identities an identity blocked (list,
    unblock; P6-21), the local name of a contact (a new operation of the
    contact store, durable before it returns), marking a contact verified,
    the request mode of an identity, the state of the Onion Service of each
    identity as a query, and a check of the Tor endpoints. Requests and
    invitations keep the identifiers of the core, which are local to one
    instance of an identity. Nothing changes on the wire or in delivery.

P6-5 Names. A contact is shown by the name the user gave it; without one,
    as "Contact" with the first group of its fingerprint. A request shows
    the display name and introduction the sender chose, as the sender's
    own words, with its fingerprint; the user names the contact on
    accepting, starting from that name. Monolith sends its own requests
    with an empty name and introduction (Phase 5), and that is kept.

P6-6 How two people become contacts is unchanged: each imports the other's
    card and the first session makes them contacts; or one imports an
    invitation card of the other, whose identity then sees a request. A
    request leaves with the first message, because the runtime dials only
    when something is queued (P5-8); the client says so where a contact
    waits.

P6-7 First start: a profile kept on disk, in the data directory of
    `STORAGE.md` section 1 under a passphrase, or a session that keeps
    nothing. Later starts ask for the passphrase.

P6-8 The composer sends with Enter; Shift+Enter makes a new line. A text
    that breaks a rule of `ChatText` or exceeds `MAX_CHAT_TEXT_LEN` cannot
    be sent and says why; the draft stays until the runtime has queued the
    message.

P6-9 A conversation is read in pages of 100 messages, newest first, at most
    2000 in memory; the view stays where the reader is unless they are at
    the newest message.

P6-10 Text: drawn as plain, selectable text. The rendering limits of
    `RESOURCE_LIMITS.md` section 10 apply; bidirectional controls and other
    invisible format characters are drawn as visible marks; text that looks
    like a URL is text.

P6-11 QR codes are shown, generated locally (`qrcodegen`); none is scanned,
    and there is no camera.

P6-12 Settings: the theme (system, light or dark), reduced motion, the Tor
    endpoints, the request mode of each identity; the location of the data
    directory is shown. Window geometry is remembered. Nothing else.

P6-13 Windows (asked by the owner during the phase: the client must work
    on Windows too). The whole stack builds and its tests pass there:
    runtime, storage (`STORAGE.md` section 3.8, std only, no check of
    access control lists), Tor and the client. Tor is that of Tor Browser
    (SOCKS 9150, control 9151, its cookie file) or of the Tor Expert
    Bundle (9050, 9051, the cookie in its data folder); Monolith never
    bundles one. The data directory is `%LOCALAPPDATA%\Monolith\Profile`,
    the settings `%LOCALAPPDATA%\Monolith\desktop.conf`. The mutation
    runner and the private Tor network of the end-to-end tests stay on
    Linux. A Tor setting is accepted only with an absolute cookie path:
    no setting turns control authentication off.

P6-14 A contact that is blocked or removed loses the name the user gave
    it: the contact store keeps no record of it (Phase 4), and adding one
    would change the vault for a label. Its conversation stays, read only,
    as "Former contact" and the first group of the fingerprint; the
    confirmation of a block or a removal says the conversation stays.

P6-15 A card or an invitation card is saved, on an explicit action, as a
    new file in the user's Downloads folder (the home folder if there is
    none), 0600 on Unix, never replacing a file; the toast names the file.
    There is no file chooser: a native one would bring GTK or a portal
    client into the binary (ADR 0010). A card is read from a path the user
    types or a file dropped on the window, at most 64 KiB, a regular file
    only.

P6-16 Tails (`PLATFORM_TAILS.md` section 5): the client keeps a profile and
    its settings only in the Persistent Storage, at
    `/home/amnesia/Persistent/monolith` and
    `/home/amnesia/Persistent/monolith-desktop.conf`, while it is mounted;
    a path given on the command line elsewhere is refused. Without it the
    first start offers only a session that keeps nothing, and the window
    says "This session saves nothing" for as long as it runs, as every
    session that saves nothing does.

P6-17 Fonts the bundled ones lack come from fixed system paths: DejaVu
    Sans and Noto Sans CJK on Linux, Segoe UI, Microsoft YaHei and Malgun
    Gothic on Windows, at most 32 MiB each. Each is checked to be readable
    before use (egui stops the process on a font it cannot read) and kept
    once for the life of the process: given to egui to own, it would be
    copied again.

P6-18 Keyboard: besides Tab, Enter, Shift+Enter and Escape, Ctrl+F (Cmd+F)
    reaches the filter and Alt+Up and Alt+Down show the conversation
    before or after, with the focus in its composer. No shortcut of the
    system is taken.

P6-19 Contrast: main text at least 7 to 1 and every other text, hints and
    times included, at least 4.5 to 1 on what it is drawn on, in both
    themes (WCAG 2.1, a test computes it). Meeting it darkened the hints
    and times of the light theme and the accent of the dark one.

P6-20 A card and an invitation are told apart (review finding B1: a
    person who added a plain card waited for ever while the other side, in
    the default mode, dropped every request without a word). Adding a card
    says whether the card the contact's record holds afterwards carries an
    invitation (for an accepted contact a card added changes nothing, so
    that need not be the card added), and a contact's summary says
    whether the card held for it carries one, which its requests carry
    (`PROTOCOL.md` sections 8.3 and 12.2). The store keeps that card in the
    contact's credentials, so nothing new is stored and the answer
    survives a restart. A contact added by a card without one is told to
    add the user's card too: in "Add contact", which stays open to say so
    with a button to the user's card, and in the conversation until the
    contact accepts. Two people who add each other's cards become contacts
    in every request mode (section 6.4), so the advice always works; that
    the other side may take requests from anyone is not known and not
    said. The own card dialog says what a card alone does under the
    identity's request mode.
P6-21 Unblocking (review finding B3: a block could not be undone in the
    client). `Messenger::blocked` lists the identities an identity blocked,
    with their fingerprints: the contact store keeps nothing else of a
    blocked identity, no name and no card. Each is named by a `BlockedRef`:
    the local identity, its instance, and the fingerprint blocked. Only
    the query makes one; `Messenger::unblock` refuses one of another
    identity or of an instance from before (the profile opened again) with
    `CommandError::NotBlocked`, as it does for an identity not blocked now.
    The handle names the identity blocked, not one block of it: giving each
    block an identifier would change the records of the contact store and
    the vault. The bridge forgets the number of someone no longer listed:
    once a read of the identity has shown them unblocked, their old number
    names nothing, also if they are blocked again. Unblocking is
    `LocalIdentity::unblock` of Phase 4, durable when it returns: the
    identity has no record at all afterwards, as one never met. It is not a
    contact again and not on the declined list; a request of its is
    considered as the request mode says (with "invitations only", only with
    an active invitation; with "nobody", never); an import of its card
    makes a requested contact with a new record and a new conversation; the
    conversation that ended with the block stays ended. Blocking again does
    not undo an unblock: with no record left, the client can block them
    again only from a request they send or once their card is added. So the
    client asks before it unblocks, saying what unblocking lets them do and
    that, and says in a toast what it did.

### 13.2 Not part of Phase 6

The brief's exclusions: the Double Ratchet, groups, attachments, voice,
video, reactions, read receipts, typing indicators, presence, push
notifications, cloud sync, a mobile client, a server, telemetry,
analytics, plugins, themes to download, scripting and remote updates.
Also not here: message search (only contacts and conversations can be
filtered), scanning QR codes, the platform adapters of Tails and Whonix.

### 13.3 Final verification and completion

The final verification ran on `68ac655bb82f4d423d46581bdc9a1816f8f87161`
from 2026-10-15 06:44 to 2026-10-18 00:44, with no file of that commit
changed while it ran: the Windows stage (Windows 11, Rust 1.95.0 and
1.85.1) in a worktree at the commit; the WSL stage (Debian 13.4) and the
Linux package in a clone at the commit; the container stage (Debian 12,
Rust 1.95.0, 1.85.1 and 1.99.0, Tor 0.4.9.13, Chutney `ae3a33c`) on an
export of the commit; and the mutation lists, two workers, in copies made
from a clone at the commit. The outputs, the pictures of every screen
among them, are on the branch `verification-68ac655`, not on `main`; the
two packages are not, and their sizes and SHA-256 are in the report. The
final report is section 6 of the status file of Phase 6 (`STATUS.md`,
removed when Phase 6 was merged), kept in the history at `6180f06`.
Results:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check`; clippy on all targets, with and without `monolith-ui/fixtures`, warnings denied | clean, on Windows and in the container |
| `cargo test --workspace --locked --features monolith-ui/fixtures` | Windows, Rust 1.95.0 and 1.85.1: 76 test runs (64 binaries and 12 of doctests), 1165 passed, 0 failed, 2 ignored, on each; Linux, Rust 1.95.0, 1.85.1 and stable 1.99.0: 76 test runs, 1167 passed, 0 failed, 2 ignored, on each. The ignored ones are the test that the private network test runs and the KDF measurement; Linux runs seven Unix-only tests, Windows five Windows-only ones |
| Rust 1.85.1: `check --workspace --all-targets` | clean, on Windows and in the container |
| `cargo deny check` 0.20.2 | advisories, bans, licenses and sources ok |
| `cargo audit` 0.22.2 | no advisory for the 445 crates |
| `cargo tree -d` | the second versions of the window stack, which `deny.toml` accepts by exact version, and `rand_core` 0.9.5 (tests only) and 0.10.1, as on `91bf22c` |
| `monolith-protocol`, `monolith-session`, `monolith-identity` | unchanged since `phase-5-complete`; no wire change |
| Fuzz, 20 targets, 26 seconds each | no crash |
| Fuzz, 600 seconds each: `ui_present` (1,218,306 runs), `ui_model_sequence` (2,407,157 runs) | no crash |
| Mutation suite, 555 faults | 546 caught, the nine expected survivors (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29), none unexpected, none invalid; the 160 faults of Phase 6, UI1 to UI160, all caught |
| `tests/network/fail-closed.sh` (T-NET-1) | pass |
| `tests/tor-network/two-node.sh`, Tor 0.4.9.13, Chutney `ae3a33c`, `RESTART_CYCLES=3` | pass, steps 1 to 21, in 790 seconds; step 21 drives two desktop clients through their window code over Tor |
| Workflows A to G by script, through the window code, the bridge and the runtime on the in-memory Tor | pass, on Windows and in the container |
| Frames | after the first, at most 1.17 ms a frame on Windows and 0.75 ms in the container; no frame drawn while nothing happens; the packaged client, idle on Windows, used 0 ms of CPU over 10 seconds |
| Packages | the Linux tar.gz and the Windows zip, each with its third-party notices, built from the commit |
| Identities of `phase-5-complete..68ac655` | 60 commits, all with `firebirks <336470935+firebirks@users.noreply.github.com>` as author and committer, no trailers |

The mutation run was interrupted four times on the same commit: by a
restart of the WSL VM from outside the verification, twice for the
owner, and once when the process that held it ended. Each time it went on
with the faults that had not ended; the Phase 5 list, stopped after 16
faults, was run again whole. Seven faults reached the runner's timeout:
MR25 and MR26, which turn a loop into one that never waits (section
12.7); MR9, CS29, CS30 and Q1, whose result files name the tests that
failed before it; and T6, which fell in a part of its list whose failing
tests were not recorded and, run again alone with one worker, made 340
named tests fail. CS29, caught by the timeout alone on `91bf22c`, now
also makes twelve tests of `tests/front_end.rs` fail, which Phase 6 added
and which runs before `tests/runtime.rs`.

After the freeze, a review of Phases 5 and 6 was checked against the code
on 2026-10-16. It found no way around authentication or the contact
rules, and nothing that changes a result of the verification. The owner
chose to close the phase on `68ac655` and to leave its findings to the
next phase (below).

The owner accepted the report and its verdict on 2026-10-18 and declared
Phase 6 complete on `68ac655`. The commits after it, up to the tag
`phase-6-complete`, change documents only and do not change the verified
code: the report (`6180f06`); the names of the Tails and Whonix adapters
in place of the phase numbers of the earlier plan (`8091073`); this
record; and the removal of the status file. From then on the mutation
faults and the regression tests of Phases 1 to 5, and UI1 to UI160, are a
baseline to preserve: none is weakened without an explicit design
decision.

Left for the next phase (item 35 of the report), besides section 13.2:

- The `activity` of a session is refreshed before the message is checked
  (`runtime.rs`): for a MessageAck before `acknowledged()`, and for a
  ChatMessage even when it is refused or is a duplicate, so a contact can
  keep an idle session open up to `MAX_SESSION_LIFETIME` (24 hours). It
  should be refreshed only by an acknowledgement that names a message in
  flight and by a message taken in.
- Reading a card file on Windows (`monolith-desktop`, `files.rs`): mapped
  network drives pass the local path check, links in intermediate
  directories are followed, online-only cloud files are read over the
  network, and the check that the opened file is the one looked at
  compares no file ID. Remote drives, reparse points along the path and
  files that are not on the disk are to be refused, the opened handle
  checked, and the save directory checked the same way (`DESKTOP.md`
  section 10).
- The runtime ignores every file message and `Profile` (the catch-all arm
  in `runtime.rs`), while `PROTOCOL.md` 13.1 makes a file message for an
  unknown id a violation. Without transfers: refuse each `FileOffer` at
  once, remember its id, and treat any other file message on an unknown
  id as a violation.
- The probe of a duplicate session: the Ping is sent just before the store
  wait of `next_to_send` (R5-28), so a live preferred session can lose its
  10-second probe; `probe_ended` does not check the credentials again.
  Liveness only.
- `is_invisible` (`monolith-ui-model`, `present.rs`) misses the tag
  characters (U+E0001, U+E0020 to U+E007F) and other format characters.
- A stale comment in `monolith-ui`, `app.rs`; and the literal at
  `monolith-ui-kit/src/theme.rs:662` that Rust 1.99.0 warns about
  (`float_literal_f32_fallback`, a lint of future incompatibility), to be
  written `2.0_f32`.
- Documents: ADR 0009 says "a fresh random nonce" where the code falls
  back to a fixed nonce if the system's random source fails, and
  describes two branches of the duplicate rule that production never
  reaches; `RESOURCE_LIMITS.md` 12 should name the write amplification a
  hostile contact can cause (each stored message rewrites the whole
  store); section 12.3 should say that ended queues also count toward
  `MAX_QUEUED_MESSAGE_BYTES`; 12.6 (R5-28) calls the store "a small
  file".
- None of the seven rates of `RESOURCE_LIMITS.md` section 6 is enforced
  yet. The per-contact rates come first; the rates of the accept path are
  hardening.
- Deferred before: tests with screen readers; shaping and bidi; a message
  list reachable by keyboard; keeping unread counts and declines; blocking
  a key without a record; the cost of the SOCKS proof for each failed
  dial; reading the Tor cookie by name; installers and signing; the
  client on Tails and Whonix, tested by hand; bounding the publication
  wait of `Peer::start` in `tests/runtime.rs` (12.8).

### 13.4 A review after the completion

On 2026-10-18, after the tag `phase-6-complete`, the code of `68ac655`
was read again against its documents, area by area: the message store,
delivery, the session manager and its scheduler, what Phase 6 added
below the window, the boundary and the files of the desktop client, and
its screens. Nothing was built or run for it. It found no way around
authentication, the contact rules or the delivery semantics, and nothing
that changes a result of the verification. Its findings are follow-up
work for the next phase, with those of 13.3; neither phase is reopened.

Medium:

- Dialing during a rotation of the peer's key: the scheduler and
  `Runtime::connect` (`runtime.rs`) dial only the first card of the dial
  plan. While an authorized successor is held that is the successor, and
  the active card is never dialed when the successor does not answer, as
  `PROTOCOL.md` sections 4 and 11.4 require; the development node of
  Phase 4 tries each card. A peer that announced a successor but still
  answers with its active key is not reached until it switches. No test
  of the runtime covers a rotation.
- A sender's display name can close its quotes: the request card
  (`monolith-ui`, `requests.rs`) and the Accept dialog (`dialogs.rs`)
  put the name inside a sentence of the client's own, and `PROTOCOL.md`
  section 9 allows `"` in a display name. The name
  `Sam". Through your invitation "Family` makes a request without an
  invitation read like one through the invitation "Family" (P6-5). The
  name belongs in an element of its own, apart from what the client says.
- Lowering a limit of the message store is not compatible:
  `RESOURCE_LIMITS.md` says lowering a local limit is a compatible
  change, but the decoder of the store refuses counts above the limits of
  its section 5, so an installation whose store holds more no longer
  opens. Either the document says so, or the limits are checked on
  writing only.

Low:

- Phase 5: `send_message` reports a shutdown as
  `SendError::UnknownIdentity`, and a failed random source or a send cut
  short at shutdown as `SendError::Storage`. `end_record` names the peer
  before its detached task starts, so "No await separates them" in 12.5
  no longer holds; `queue_message` does not look at the record again
  after its wait for the vault either (the outcome is that of the other
  order). After its session ends because the contact was blocked or
  deleted, the scheduler still raises `DeliveryDeferred` with a retry
  that never comes. A Pong that arrives after the probe's deadline ends
  a live preferred session as a violation. `ServiceState::NoKey` is never
  reported: `Stopped` replaces it at once. The jitter `u8 % 101` is not
  uniform, as `RESOURCE_LIMITS.md` section 7 says it is. Decoded message
  texts are not erased from memory; no document promises it.
- Documents of the store: ADR 0008 "Writing" contradicts itself on
  removing a `messages.new` without a `messages`, and `STORAGE.md` 3.5
  leaves that case out; the comment of `MessageFile::recreate` is stale;
  `OpenError::AuthenticationFailed` says a wrong passphrase and a change
  cannot be told apart, but the store is opened only after the vault
  authenticated; `RESOURCE_LIMITS.md` section 11 probably understates the
  memory of a write of the store (not measured).
- Tor and Windows: with the SOCKS proof on, "Check Tor" says Tor is not
  running when only the control endpoint fails. The cookie file Tor
  names is compared with the configured one case-sensitively on Windows,
  so a path typed in another case is refused (it fails closed). The
  rename of `vault.new` is tried once, so a scanner that holds the file
  stops the installation until a restart (whether that happens was not
  established).
- Desktop: a card path that names a device (`C:\COM3`) is opened by the
  look at its metadata before it is refused (not established on Windows
  11). A cookie path, `--data-dir` or `--config` on a network share
  (`\\host\share`) is accepted, and reading it reaches that host outside
  Tor. The saved size of the window is divided by the zoom. The window
  thread looks at the disk for the folder of a saved card and for the
  marks of Tails and Whonix. A contact of another identity accepting
  shows a toast over the identity shown (ADR 0011, State). On Tails a
  refused `--data-dir` is explained as storage to unlock, and a refused
  `--config` is not explained.
- Screens: `MAX_RENDERED_UNBROKEN_RUN` is not applied. White initials on
  five of the eight light colours of avatars are below 4.5 to 1 (P6-19),
  and no test looks at them. `present::MAX_NAME_BYTES` is documented as
  `MAX_DISPLAY_NAME_LEN` (64 against 128) and is not used.
- Not established: whether an Enter during the composition of an input
  method reaches the composer before the text is committed.

## 14. Phase 7: security and protocol hardening

Phase 7 began on 2026-10-18 from `main` at `20a6779` (`phase-6-complete`
and two documentation commits; the verified commits of Phases 5 and 6 are
`91bf22c` and `68ac655`), on the branch `phase-7-security-hardening`. Its
brief: close the backlog of sections 13.3 and 13.4 and the deferred items
of earlier phases they name, preserving every invariant verified before.
No new user-facing feature. Each item ends fixed, shown not to be a
defect, or deferred with the owner's approval. The inventory, BL1 to
BL53, is in the status file of the phase.

### 14.1 Decisions

None yet.

### 14.2 Not part of Phase 7

File transfer and attachments, the Double Ratchet, groups, a mobile
client, voice and video, read receipts, typing indicators, presence, push
notifications, cloud sync, a new transport, a new identity model. Where a
fix needs Monolith to refuse a message it does not support, it does only
that.
