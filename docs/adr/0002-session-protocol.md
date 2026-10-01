# ADR 0002: Session protocol

Status: not decided. Two finalists remain, D (TLS 1.3 with raw public keys)
and F (Noise XK with a transport key certified in the contact card). This
record recommends F and gives the reasons; the choice is the project
owner's. No session code exists.
Date: 2026-10-01

An earlier revision of this record selected D. A review of that revision
asked for one more candidate to be compared before anything is frozen: a
Noise pattern in which the responder's static key is known in advance and
is bound to the identity by the signed contact card. That candidate is F
below. It is a concrete form of what the earlier revision listed as C and
set aside too quickly.

## Context

Two peers connected over a Tor stream need mutual authentication of their
Monolith identities, a confidential channel with forward secrecy, and
resistance to replay and relay. Tor already encrypts Onion Service traffic
end to end. The session layer is defense in depth, and it is what ties a
connection to an identity that is independent of the onion address.

Fixed points, whatever construction is chosen:

- nothing home-made: no custom key exchange, cipher or key schedule;
- the least possible security-critical composition that Monolith owns;
- the Monolith identity is an Ed25519 key that is distinct from the Onion
  Service key (ADR 0001);
- an initiator does not reveal its identity to an endpoint that has not
  proved the identity it dialed;
- bounded parsing in front of unauthenticated peers;
- nothing sent to a peer that describes the platform or the build beyond
  what the chosen library's handshake unavoidably shows.

Not required: deniability, post-quantum security, early data, session
resumption, group security. Nothing is added for them.

## Candidates

### A. Noise XX, per-connection static keys, signature over the handshake hash

`Noise_XX_25519_ChaChaPoly_SHA256`. Each side generates a new static key
for every connection. After the handshake each side sends, inside the
channel, an Ed25519 signature by its identity key over the handshake hash
and a few other fields (the AuthProof of the earlier drafts).

Noise provides a forward-secret channel between the holders of two
single-use static keys that stand for nothing. All long-term authentication
comes from a signature construction of Monolith's own. The Noise
specification mentions this use in one sentence (section 11.2, channel
binding) and does not analyze it. The static keys exist for a side effect:
an initiator has to know the responder's identity key, through the
prologue, to be handed a proof.

A variant, A2, is what libp2p specifies: sign the Noise static public key
with the identity key and send the signature in the handshake payload. The
signature covers nothing but the static key, so whoever obtains one static
private key together with the public signature holds a credential for the
identity that never expires.

Not pursued. The authentication is Monolith's composition, and the static
keys carry no meaning.

### B. Noise NN and signatures over the handshake hash

`Noise_NN_25519_ChaChaPoly_SHA256`, then the same signatures as in A. More
honest than A about what Noise provides. It is TLS 1.3's authentication
written again by hand, without its analysis, and the responder proves its
identity to any party that connects.

Not pursued.

### C. Noise with a persistent static key bound to the identity

Each identity holds a long-lived X25519 key that the Ed25519 identity
certifies. The earlier revision described two forms, the certificate in the
handshake payload (C1, libp2p with a stored key) and the static key in the
contact card (C2), and rejected both for needing a new credential with
lifetime rules of its own. That was too coarse for C2: the contact card is
already a signed, versioned statement with an epoch, and a transport key
can be one more field of it. F is that design, worked out.

### D. TLS 1.3 with raw public keys on both sides

TLS 1.3 with RFC 7250 raw public keys. The Ed25519 identity key is the TLS
authentication key of each side. The initiator accepts exactly one key, the
identity it dialed. The responder accepts any valid key and leaves the
question of who that is to the layers above. Detailed below.

### E. Other constructions

Other maintained Noise implementations in Rust (`clatter` 2.3.0,
`noise-protocol` 0.2.1 with `noise-rust-crypto` 0.6.2) have no audit and no
known deployment of note. They change the library under a Noise option, not
the comparison. No other session stack was found that is mature, reviewed
and a better fit.

### F. Noise XK or IK with a transport key certified in the contact card

Each identity has three keys with three jobs:

    identity key    Ed25519   signs contact cards, nothing else
    transport key   X25519    authenticates sessions
    onion key       Ed25519   reachability, used by Tor

The contact card, which the identity key signs, states the transport key
next to the endpoints, under the same epoch. A party that holds a card
therefore knows the responder's transport key before it connects, which is
the situation the Noise patterns with a known responder key are made for.
Noise authenticates the transport keys. The card says whose they are. No
signature is made during a session.

The review proposed IK. Both IK and XK fit; they differ in when the
initiator's key is sent.

    IK   -> e, es, s, ss        the initiator's key in the first message
         <- e, ee, se

    XK   -> e, es               the initiator's key in the third message,
         <- e, ee               after the responder has answered
         -> s, se

IK saves a message and pays for it. Its first message carries the
initiator's static key encrypted to the responder's static key without
forward secrecy, and it can be replayed. The Noise specification rates the
hiding of the initiator's identity 4 for IK ("if an attacker learns the
responder's private key they can decrypt the initiator's public key") and 8
for XK ("encrypted with forward secrecy to an authenticated party").
Concretely: someone who holds the onion key but not the transport key can
answer at the address and record first messages. With IK, a transport key
obtained later reveals who tried to connect. With XK there is nothing to
decrypt, because the initiator sends its key only after the responder has
proved that it holds the transport key. Monolith does not need the round
trip IK saves and does require that order. F is therefore XK.

## Comparison

"Composition" means security-critical protocol logic that Monolith would
specify itself and that no standard or published analysis covers.

| | A: XX + signature over h | B: NN + signature over h | D: TLS 1.3 raw public keys | F: XK, transport key in the card |
| --- | --- | --- | --- | --- |
| Cryptographic maturity | X25519, ChaCha20-Poly1305, SHA-256, Ed25519 | same | same primitives, in the TLS 1.3 key schedule | X25519, ChaCha20-Poly1305, SHA-256; Ed25519 for cards only |
| Protocol maturity | Noise revision 34 (2018), "official/unstable"; the signature step is outside it | same | RFC 8446 (2018), now RFC 9846 (2026); RFC 7250 | Noise revision 34; XK as specified; certificates for static keys are the method the specification names (section 14) |
| Implementation maturity | `snow` 0.10.0 (2025-07); one owner, "reasonable-effort", no release since | same | `rustls` 0.23.45 (2026-09); a team, frequent releases | `snow` 0.10.0, as A |
| Audit history | `snow`: Trail of Bits, January 2024, earlier commit; ten findings, eight fixed, two open (keys are not cleared). The proof: none | same | `rustls`: Cure53, 2020; four low or informational findings. Nothing later found | `snow` as A. The binding rules: none |
| Known deployments | a signature over h: one found | same | TLS 1.3 everywhere; raw public keys with `rustls`: iroh, over QUIC | XK with a static key published in a signed record: I2P NTCP2; Lightning (XK with the node key); Tor's ntor handshake has the same shape for one side |
| MSRV | 1.85 | 1.85 | `rustls` 1.71, `ring` 1.66 | 1.85 |
| Licence | Apache-2.0 OR MIT | same | Apache-2.0 OR ISC OR MIT; `ring` Apache-2.0 AND ISC | Apache-2.0 OR MIT |
| Transitive footprint (measured) | 23 crates more than today | same | 17 crates more than today | 23 with the stock resolver; fewer with a resolver over current crates |
| Unsafe and non-Rust code | `snow` forbids unsafe | same | `rustls` forbids unsafe; `ring` contains C and assembly | `snow` forbids unsafe |
| Long-term maintenance | one maintainer; key erasure requested since 2017 | same | `rustls` well staffed, 0.24 and 1.0 planned for late 2026; `ring` without a release since 2025-03 | as A |
| Forward secrecy | yes | yes | yes; resumption and early data disabled | yes |
| Mutual authentication | by Monolith's signatures | same | by TLS CertificateVerify | by Noise, of the transport keys; the card binds them to identities |
| Responder authentication | after the handshake | same | first server flight | second message |
| Initiator identity privacy | hidden from all but a responder that proved the dialed identity | same | same, RFC 9846 appendix F.1 | same, with forward secrecy; Noise rating 8 |
| Responder identity privacy | shown to a prober that knows the identity key | shown to any prober | shown to any prober, with a signature | never transmitted; Noise rating 3 |
| Resistance to active probing | friction: a prober needs the public identity key | none | none: a prober gets the identity key and a fresh signature over a transcript it chose part of | a prober without the card gets nothing, not even a reply; with the card it learns that the key holder is live |
| Transcript binding | signature over h, by Monolith | same | CertificateVerify and Finished, by TLS | Noise handshake hash; every message is authenticated under it |
| Replay resistance | fresh ephemerals in h | same | fresh randoms and key shares | fresh ephemerals; a replayed first message gets a reply nobody can use |
| Downgrade resistance | one suite | same | one version, suite and group configured; negotiation covered by the transcript | one suite; the version is in the prologue |
| Reflection and role confusion | role byte, by Monolith | same | context strings, by TLS | roles are fixed by the pattern |
| Key separation | identity key signs under a Monolith prefix | same | identity key signs cards and, online at every handshake, TLS transcripts | identity key signs cards only and is not needed for a session |
| Key rotation | none needed | none needed | none needed | a new transport key is a new card epoch; no revocation |
| Session resumption | none exists | none exists | exists; disabled | none exists |
| 0-RTT | none exists | none exists | exists; disabled | not with XK |
| Implementation complexity | small | smallest | configuration, two verifiers, a signer | small: fixed messages, card checks |
| Monolith-owned composition | the proof | the proof | no cryptographic construction; the verifiers, the signer and the configuration are Monolith's | five binding rules, listed below |
| Parser before authentication | 192 bytes, fixed | fixed, less | the TLS handshake parser: variable messages and extensions, up to 64 KiB per message | 48 bytes, fixed; then 48 and 235 |
| Fuzzability | glue is small | same | glue is small; `rustls` is fuzzed upstream | glue is small |
| Test vectors | deterministic handshakes through a test hook in `snow` | same | a deterministic handshake needs a test-only provider | as A; Noise vectors exist for XK |
| Zeroization | `snow` clears nothing | same | `rustls` clears what it holds; `ring` does not | `snow` clears nothing, and here it would hold a long-term key |
| Duplicate dependencies | a second `curve25519-dalek` and `sha2`, or a resolver of our own | same | `getrandom` 0.2 next to 0.3 | as A |
| Tails and Whonix | pure Rust | pure Rust | needs a C compiler; Debian 13 packages older versions | pure Rust |

## The two finalists

A and B are out: both leave the long-term authentication to a construction
of Monolith's own with nothing gained in return. D and F each avoid that,
in different ways.

### What decides between them

| | D: TLS 1.3 raw public keys | F: Noise XK, key in the card |
| --- | --- | --- |
| Bytes an unauthenticated peer can make the responder parse | a ClientHello with extensions, then encrypted handshake messages; bounded by a cap of ours and by the library | 48, of fixed layout |
| Code reachable before authentication | a general TLS state machine and its parsers | one Diffie-Hellman operation and one tag check |
| What a prober without the card gets | the identity key and a signature | nothing |
| What a prober with the card gets | the same, as transferable evidence that the key answered | knowledge that the key holder is live; nothing it can show to others |
| Use of the identity private key | online, one signature per handshake | only when a card is issued |
| Authentication rests on | TLS 1.3, analyzed in many models | Noise XK, analyzed symbolically and computationally, and five rules of ours |
| What Monolith has to get right | verifiers, signer, a dozen configuration choices, checks after the handshake | the five rules |
| Library | well staffed, audited in 2020, three advisories in three years, interface about to change | one maintainer, audited in 2024, one advisory, clears no keys |
| Long-term secrets | one | two |
| Change to Phase 1 formats | none to cards; AuthProof and preamble removed; frame overhead 0 | contact card gains a field; AuthProof and preamble removed; frame overhead stays 16 |
| Measured handshake, both sides, one machine | 0.31 ms; 203, 368 and 193 bytes | 0.44 ms; 48, 48 and 235 bytes |

### The rules F needs

Certifying a Diffie-Hellman key with a signing key is the method the Noise
specification names for exactly this situation. It is still composition,
and it is where F can go wrong. The rules, with the reason for each:

F1. The card carries the transport key, and the card signature covers it.
    The card is the certificate. No second signed structure is introduced.

F2. The Noise prologue contains the responder's identity key. Without it
    there is an attack. Mallory signs a card of her own that names Bob's
    endpoint and Bob's transport key. Alice imports it, connects, reaches
    Bob, and Noise succeeds, because Bob does hold that transport key.
    Alice now attributes Bob's messages to Mallory. Nothing in Noise
    prevents this: a certificate says the identity vouches for the key,
    and nothing says the key holder accepts the identity. With the
    identity key in the prologue, Alice hashes Mallory's key and Bob his
    own, and the first message fails. This is the misbinding that the
    SIGMA paper describes for certified Diffie-Hellman keys. Tor's ntor
    handshake mixes the relay identity into its key derivation for the same
    reason.

F3. The initiator sends its own contact card, without capability, as the
    payload of the third message. The responder checks the card signature
    under strict rules, the validity of the identity key, that the
    identity is not its own, and that the transport key in the card is
    byte for byte the static key Noise authenticated. Only then is the
    initiator's identity the one in the card. The card is inside the
    handshake, so it is the key holder who presents it.

F4. A card presented in a handshake whose epoch is lower than the one the
    responder has pinned for that identity does not make the session a
    contact session. The peer is treated as any identity that is not a
    contact, with the same generic Close. This keeps a stolen, retired
    transport key from being used against contacts who know the newer one,
    without telling the peer why. Against a responder that has never seen
    the identity there is no defense: version 1 has no revocation, for
    transport keys as for identity keys.

F5. A transport key of small order is invalid, in a card and in a
    handshake, and a Diffie-Hellman result of all zeros ends the
    handshake. `snow` checks neither.

A review of the earlier revision, which selected D, is a useful measure of
what "no composition" is worth. It found one contradiction in the
specification that would have led an implementer to send the initiator's
identity before verifying the responder, and half a dozen cases the
specification did not cover: a HelloRetryRequest, a responder that sends no
CertificateRequest, the signature algorithm field, KeyUpdate, the alerts a
library sends on its own. None is a flaw of TLS. Each is something Monolith
has to specify and test. D has no cryptographic construction of ours, but
its glue is not smaller than the five rules above.

### Assessment

F is recommended, as XK with the five rules.

1. The attack surface in front of unauthenticated peers is the smallest of
   any candidate: 48 bytes of fixed layout, one Diffie-Hellman operation,
   one tag. The listener is reachable by anyone who has a contact card.
   Monolith has no update mechanism by design, so a parser error in a
   large dependency stays reachable until the user upgrades by hand.
2. It gives nothing to a prober. D hands every caller the identity key and
   a signature over a transcript the caller contributed to, which the
   caller can show to others as evidence that the key was online. F sends
   no signature in a session at all.
3. The identity key leaves the session path. It signs cards and nothing
   else, and can stay locked while the application runs. In D it signs
   whenever anyone connects.
4. The authentication is that of Noise XK, which has symbolic and
   computational analyses. What Monolith adds is a certificate, in the
   form the Noise specification suggests and I2P has deployed since 2018.
5. The frame layer of Phase 1 stays as it is.

What is accepted with F:

- Five rules of Monolith's own, reviewed by nobody outside the project.
  F2 shows how such a rule is missed. They are few, each is stated with
  its reason, and each gets tests that fail when it is removed.
- A change to the contact card before anything is deployed: one more
  field, new test vectors, a longer text form.
- A second long-term secret in the vault and in every backup.
- `snow`: one maintainer, no release in fourteen months, and no erasure of
  keys. With F the transport key is long-lived, so the last point weighs
  more than it did for A. See F-R1.
- No revocation. A new transport key reaches a contact only when the two
  next talk.

D remains a sound choice if the owner prefers to own no binding rules at
all and accepts the larger parser, the disclosure to probers and the
identity key on the session path. Its specification is drafted in
`PROTOCOL.md` and `CRYPTOGRAPHY.md`; the corrections it needs are listed
under "Finalist D in detail".

## Finalist F in detail

This section is a design, at the level needed to judge it. If F is chosen,
`PROTOCOL.md` and `CRYPTOGRAPHY.md` are rewritten from it and the exact
bytes are fixed there.

Pattern and suite: `Noise_XK_25519_ChaChaPoly_SHA256`.

Prologue, 51 bytes: the 19 ASCII bytes `MONOLITH-SESSION-V1` followed by
the responder's identity public key. The label carries the protocol
version. The initiator takes the identity key from the contact it dials;
the responder uses its own.

Pre-message: the responder's static key is the transport key in the card
the initiator holds for that identity.

    Initiator                                   Responder

      e, es                    48 bytes  ---->
                                                checks the tag: the caller
                                                  knows my transport key
                                                  and my identity key
                               48 bytes  <----  e, ee
      checks the tag: the peer holds
        the transport key of the
        identity I dialed
      s, se, card             235 bytes  ---->
                                                checks rules F3 to F5

      [frames]                           <--->  [frames]

Sizes are those of the prototype, with a card of 171 bytes.

Order. The responder proves possession of its transport key in the second
message. The initiator sends its transport key and its card only after
that, encrypted with forward secrecy.

What each party learns:

| Party | Learns |
| --- | --- |
| Passive observer of the stream | Three messages of fixed size. Nothing readable. |
| Active party without the responder's card | Nothing. Its first message fails the tag and the responder closes without replying. |
| Active party with the responder's card | That the holder of the transport key is live at the address. No signature and nothing transferable. |
| Party that answers at the address without the transport key | 48 bytes it cannot use. The initiator stops at the second message and has sent no identity. Obtaining the transport key later reveals nothing from what was recorded. |
| Authenticated responder | The initiator's card: identity, transport key, endpoints. |

The last row is not a new disclosure. An initiator dials only identities
it holds as contacts or has asked to become contacts, and both receive its
card anyway.

Contact card. One field is added to the signed bytes and to the binary
form: the 32-byte transport key. A card grows from 139 to 171 bytes, 187
with a capability. The epoch covers the transport key as it covers the
endpoints: a card with the same epoch and another transport key is a
conflict, and a new transport key needs a greater epoch.

Transport. Frames are Noise transport messages, as Phase 1 assumes: a
16-byte tag per frame, nonces counting from zero, a frame that fails
authentication ends the session. Replay, loss and reordering of frames are
detected by Noise. Limits per session are unchanged: 24 hours, 2^32
frames, 2^40 bytes, far below the 2^64 nonces of a cipher state, and there
is no rekey.

Invitation capability. Unchanged: inside the ContactRequest, compared in
constant time, looked at after the Close. It can be checked only after a
complete handshake, which costs a responder three Diffie-Hellman
operations and one signature verification.

Key lifecycle:

| Secret | Generated | Lives | Stored | Erased |
| --- | --- | --- | --- | --- |
| Identity private key | by Monolith | until the identity is discarded | vault | by its type; needed only to issue a card |
| Transport private key | by Monolith, independently | until replaced by a new card epoch | vault | by its type; inside `snow` only if the resolver is ours |
| Noise ephemeral key | by the Noise library | one handshake | never | not by `snow` |
| Chaining key, cipher keys | by Noise | one session | never | not by `snow` |
| Invitation capability | by Monolith | until revoked | vault | by its type |

The transport key is not derived from the identity key or from a shared
seed.

Consequences for Phase 1:

- the contact card format changes as above, with new test vectors and fuzz
  seeds; ContactRequest and EndpointUpdate grow by 32 bytes and still fit
  one padding block before confirmation;
- the preamble and AuthProof are removed, with the feature bits; message
  code 0x0001 becomes unassigned;
- the handshake record sizes become 48, 48 and 235;
- the frame format, the frame overhead of 16 and every other message are
  unchanged;
- the session logic is told by the handshake which identity was
  authenticated, as with D.

Risks and open points of F:

- F-R1. Library and provider. Three ways to run XK through `snow`: its
  stock resolver, which brings a second `curve25519-dalek`, `sha2` and
  `chacha20poly1305` and leaves the transport key in memory it never
  clears; its `ring` resolver, which has no X25519 and so cannot serve
  this suite alone; or a resolver of Monolith's own over `x25519-dalek`
  3, `chacha20poly1305` 0.11 and `sha2` 0.11, as libsignal and
  rust-libp2p do, which keeps the transport key in an erasing type and
  can enforce F5, at the cost of security-sensitive glue of ours that has
  to pass the Noise test vectors. Avoiding duplicate crates is not a
  reason for the third; erasing a long-term key and rejecting degenerate
  results are. To be decided with evidence before code, in this record.
- F-R2. The five rules have no external review.
- F-R3. `snow` may stop being maintained. The Noise handshake is small
  and specified; replacing the library is contained in one crate.
- F-R4. No revocation of a transport key.
- F-R5. A stale card presented to a responder that never saw a newer one
  is accepted. Same root as F-R4.

## Finalist D in detail

- Protocol: TLS 1.3 (RFC 9846, which replaced RFC 8446 in July 2026 without
  changing what is used here) with raw public keys (RFC 7250) in both
  directions.
- Library: `rustls` 0.23.45, pinned exactly, features `ring` and `std`.
  Provider: `ring` 0.17.14. Suite `TLS_CHACHA20_POLY1305_SHA256`, group
  x25519, signature scheme ed25519. ALPN identifier `monolith/1`.
- The TLS end-entity key of each side is its Monolith identity key, as the
  44-byte Ed25519 SubjectPublicKeyInfo. Signatures are made and verified
  with `ed25519-dalek` through the library's interfaces.

The handshake, the 130-byte signed input of CertificateVerify, the checks
of each side and what each party learns are specified in `PROTOCOL.md`
sections 3, 4 and 6 and `CRYPTOGRAPHY.md` sections 4 to 8.

    Initiator (TLS client)                      Responder (TLS server)

      ClientHello                       ---->
                                                ServerHello
                                                {EncryptedExtensions}
                                                {CertificateRequest}
                                                {Certificate}
                                                {CertificateVerify}
                                        <----   {Finished}
      verifies the responder
      {Certificate}
      {CertificateVerify}
      {Finished}                        ---->
                                                verifies the initiator
      [frames]                          <--->   [frames]

The message order is TLS's. That the initiator applies its pin before it
sends its Certificate is the behavior of the verifier and the library; the
prototype confirms it for `rustls`.

Corrections the drafted specification needs if D is chosen, from the review
of the earlier revision:

- The checks on the peer's key and signature run inside the handshake,
  before the initiator's own flight. `IdentityAuth` applies only the checks
  after Finished. One sentence in the earlier text said otherwise and has
  been corrected.
- Alerts. Rejections that come from Monolith's verifiers use one alert.
  The library sends others on its own, for example for an empty client
  certificate. None depends on what the responder holds about the
  initiator. RFC 9846 asks for `decrypt_error` on a bad signature; the
  profile has to say which it uses.
- The signature algorithm in CertificateVerify must be checked to be
  ed25519; the library hands it to the verifier unchecked.
- A responder that sends no CertificateRequest, and a HelloRetryRequest,
  have to be specified. A side that receives a KeyUpdate request must
  answer it.
- The signer handed to the library should refuse any input that is not
  the 130-byte TLS form for its role, so that "the identity key signs two
  kinds of input" does not depend on the library.
- Session limits belong in `PROTOCOL.md`, with the unit that is counted.
- Statements that the handshake proves the identity "live at this
  address" are too strong: the session is not bound to the onion address
  (`PROTOCOL.md` open question P2), so a relay that forwards bytes is not
  detected. This applies to F as well.
- A TLS client cannot know that the server accepted its authentication
  until the server sends application data (RFC 9846 appendix F.1.2).
- Leftovers of the earlier draft in `TEST_PLAN.md`, `RESOURCE_LIMITS.md`,
  `ARCHITECTURE.md` and ADR 0003 that still describe a 16-byte overhead
  or an identity proof.

Risks of D: the size of the parser in front of unauthenticated peers; the
future of `ring`; the announced change of the `rustls` interface, with
0.23.45 as the oldest acceptable version because of RUSTSEC-2026-0285; an
audit from 2020; no deterministic handshake without a test provider;
disclosure of the identity key and a signature to probers; unmeasured
memory per unfinished handshake.

Consequences of D for Phase 1: the preamble, the handshake records and
AuthProof are removed; the frame overhead becomes 0 and the frame length
field 16 less; contact cards and the published test vectors do not change.

## Common to both finalists

- Authentication and authorization stay separate. A session authenticates
  an identity. Whether that identity is a contact is decided afterwards by
  `PROTOCOL.md` section 6.4, and a responder's handshake never depends on
  it.
- An outbound session accepts only the identity that was dialed. Anything
  else is `IdentityMismatch`, with no option to continue.
- One suite, no negotiation, no resumption, no early data, no rekey.
- The invitation capability is checked after the handshake, in the
  ContactRequest.
- One crate holds every call to the session library. A handshake object
  cannot send or receive frames; only the authenticated session it yields
  can, and that object owns the session logic of the protocol core.
- AuthProof, the preamble and the feature bits of the earlier drafts are
  removed. Message code 0x0001 becomes unassigned.
- `native-tls` and `openssl-sys` stay banned. Neither finalist needs them.
- Monolith's own part of either design has not been audited.

## Public keys are not secrets

Knowing a responder's public keys is not possession of a secret and is
never access control, authorization or authentication of the initiator. In
F a caller needs the responder's identity key and transport key to get any
reply. Both are in every contact card. What F provides is that a party
with the onion address alone learns nothing, and that nobody is handed a
signature. It keeps nobody out who holds a card.

If Monolith ever requires a real secret before it answers, that secret has
to be a secret, such as the invitation capability. Noise has a place for
one, a pre-shared key. It is not used in version 1.

## Sources

Accessed 2026-10-01. Measurements were made with throwaway prototypes that
are not part of the repository.

- TLS 1.3: https://www.rfc-editor.org/rfc/rfc9846 , which obsoletes
  https://www.rfc-editor.org/rfc/rfc8446
- Raw public keys: https://www.rfc-editor.org/rfc/rfc7250
- Ed25519 in SubjectPublicKeyInfo: https://www.rfc-editor.org/rfc/rfc8410
- rustls: https://docs.rs/rustls/0.23.45 ,
  https://github.com/rustls/rustls/releases ,
  https://github.com/rustls/rustls/issues/2400
- rustls audit: https://github.com/rustls/rustls/blob/main/audit/TLS-01-report.pdf
- rustls advisories: https://rustsec.org/packages/rustls.html
- ring: https://github.com/briansmith/ring/discussions/2414
- iroh: https://docs.rs/crate/iroh/1.3.0/source/src/tls.rs
- Noise specification, revision 34: https://noiseprotocol.org/noise.html
  (7.7 payload properties, 7.8 identity hiding, 14 application
  responsibilities)
- Noise Explorer: https://eprint.iacr.org/2018/766 ,
  https://noiseexplorer.com/patterns/XK/
- fACCE analysis of Noise, including XK: https://eprint.iacr.org/2019/436
- A Spectral Analysis of Noise:
  https://www.usenix.org/conference/usenixsecurity20/presentation/girol
- SIGMA: https://www.iacr.org/archive/crypto2003/27290399/27290399.pdf
- Tor ntor handshake:
  https://spec.torproject.org/proposals/216-ntor-handshake.html
- I2P NTCP2: https://i2p.net/en/docs/specs/ntcp2
- Lightning transport: https://github.com/lightning/bolts/blob/master/08-transport.md
- snow: https://github.com/mcginty/snow , https://docs.rs/snow/0.10.0 ,
  https://rustsec.org/advisories/RUSTSEC-2024-0011.html
- snow audit:
  https://github.com/trailofbits/publications/blob/master/reviews/2024-03-agilebits-snow-securityreview.pdf
- libsignal's resolver for snow:
  https://github.com/signalapp/libsignal/blob/main/rust/attest/src/snow_resolver.rs
- libp2p Noise: https://github.com/libp2p/specs/blob/master/noise/README.md
- clatter: https://github.com/jmlepisto/clatter
- noise-rust: https://github.com/blckngm/noise-rust
