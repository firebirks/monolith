# ADR 0002: Session protocol

Status: decided. The session layer is Noise XK with a transport key that
the identity certifies in the contact card, run through `snow` with a
crypto resolver of Monolith's own over the current RustCrypto and dalek
crates.
Date: 2026-10-01. Amended 2026-10-02 by the credential binding review:
rules F2 and F4 and the change of transport key (`DESIGN_QUESTIONS.md`
section 8). Amended 2026-10-05 by Phase 4: F4 is applied by one decision
whatever path a card took, and the initiator's third message is made only
in the admission of the responder (`DESIGN_QUESTIONS.md` P4-1, P4-2). The
construction, the wire format and the test vectors are unchanged.

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
- nothing sent to a peer that describes the platform or the build.

Not required: deniability, post-quantum security, early data, session
resumption, group security. Nothing is added for them.

History of this record. Phase 0 proposed Noise XX with per-connection
static keys and a signed identity proof (A below). A first revision of this
record selected TLS 1.3 with raw public keys (D). The review of that
revision pointed at the size of the TLS parser in front of unauthenticated
peers and asked for one more candidate: a Noise pattern in which the
responder's static key is known in advance and is bound to the identity by
the signed contact card (F). F was compared with D and selected.

## Decision

- Protocol: `Noise_XK_25519_ChaChaPoly_SHA256`, Noise Protocol Framework
  revision 34. One suite, nothing negotiated.
- Keys: every identity has an X25519 transport key, separate from its
  Ed25519 identity key and from its onion keys, and generated
  independently of both. It is the Noise static key.
- Identity binding: the contact card states the transport key and is
  signed by the identity key. The responder's identity key is in the Noise
  prologue. The initiator presents its own card in the third handshake
  message. Five rules, F1 to F5 below.
- There is no identity proof message and no signature during a session.
- Order: the responder is authenticated first, in the second message. The
  initiator reveals its transport key and identity in the third.
- Library: `snow` 0.10.0, without its default features.
- Crypto provider: a resolver in the session crate over `x25519-dalek`
  3.0.0, `chacha20poly1305` 0.11.0 and `sha2` 0.11.0, with randomness
  from `getrandom`. F-R1 below gives the reasons.

`PROTOCOL.md` sections 3, 4, 6, 10.2 and 11 give the bytes.
`CRYPTOGRAPHY.md` gives the properties and the key lifecycle.

## Candidates

### A. Noise XX, per-connection static keys, signature over the handshake hash

`Noise_XX_25519_ChaChaPoly_SHA256`. Each side generates a new static key
for every connection. After the handshake each side sends, inside the
channel, an Ed25519 signature by its identity key over the handshake hash
and a few other fields.

Noise provides a forward-secret channel between the holders of two
single-use static keys that stand for nothing. All long-term authentication
comes from a signature construction of Monolith's own. The Noise
specification mentions this use in one sentence (section 11.2) and does not
analyze it.

A variant, A2, is what libp2p specifies: sign the Noise static public key
with the identity key and send the signature in the handshake payload. The
signature covers nothing but the static key, so whoever obtains one static
private key together with the public signature holds a credential for the
identity that never expires.

Rejected. The authentication is Monolith's composition, and the static
keys carry no meaning.

### B. Noise NN and signatures over the handshake hash

`Noise_NN_25519_ChaChaPoly_SHA256`, then the same signatures as in A. More
honest than A about what Noise provides. It is TLS 1.3's authentication
written again by hand, without its analysis, and the responder proves its
identity to any party that connects.

Rejected.

### C. Noise with a persistent static key bound to the identity

The general form of F. An earlier revision rejected it for needing a new
credential with lifetime rules of its own. That was too coarse: the contact
card is already a signed, versioned statement with an epoch, and a
transport key can be one more field of it.

### D. TLS 1.3 with raw public keys on both sides

TLS 1.3 (RFC 9846, formerly RFC 8446) with RFC 7250 raw public keys through
`rustls` 0.23.45 and `ring`. The Ed25519 identity key is the TLS
authentication key of each side. The initiator accepts exactly one key, the
identity it dialed.

Rejected, for the reasons under "Why F and not D".

### E. Other constructions

Other maintained Noise implementations in Rust (`clatter` 2.3.0,
`noise-protocol` 0.2.1 with `noise-rust-crypto` 0.6.2) have no audit and no
known deployment of note. They change the library, not the comparison. No
other session stack was found that is mature, reviewed and a better fit.

### F. Noise XK with a transport key certified in the contact card

Each identity has three keys with three jobs:

    identity key    Ed25519   signs contact cards, nothing else
    transport key   X25519    authenticates sessions
    onion key       Ed25519   reachability, used by Tor

The contact card states the transport key next to the endpoints, under the
same epoch. A party that holds a card knows the responder's transport key
before it connects, which is the situation the Noise patterns with a known
responder key are made for. Noise authenticates the transport keys. The
card says whose they are.

XK and not IK. Both patterns fit.

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
trip IK saves and does require that order.

## Comparison

"Composition" means security-critical protocol logic that Monolith
specifies itself and that no standard or published analysis covers.

| | A: XX + signature over h | B: NN + signature over h | D: TLS 1.3 raw public keys | F: XK, transport key in the card |
| --- | --- | --- | --- | --- |
| Cryptographic maturity | X25519, ChaCha20-Poly1305, SHA-256, Ed25519 | same | same primitives, in the TLS 1.3 key schedule | X25519, ChaCha20-Poly1305, SHA-256; Ed25519 for cards only |
| Protocol maturity | Noise revision 34 (2018), "official/unstable"; the signature step is outside it | same | RFC 8446 (2018), now RFC 9846 (2026); RFC 7250 | Noise revision 34; XK as specified; certificates for static keys are the method the specification names (section 14) |
| Implementation maturity | `snow` 0.10.0 (2025-07); one owner, "reasonable-effort", no release since | same | `rustls` 0.23.45 (2026-09); a team, frequent releases | `snow` 0.10.0, as A |
| Audit history | `snow`: Trail of Bits, January 2024, earlier commit; ten findings, eight fixed, two open (keys are not cleared). The proof: none | same | `rustls`: Cure53, 2020; four low or informational findings. Nothing later found | `snow` as A. The binding rules: none |
| Known deployments | a signature over h: one found | same | TLS 1.3 everywhere; raw public keys with `rustls`: iroh, over QUIC | XK with a static key published in a signed record: I2P NTCP2; Lightning (XK with the node key); Tor's ntor handshake has the same shape for one side |
| MSRV | 1.85 | 1.85 | `rustls` 1.71, `ring` 1.66 | 1.85 |
| Licence | Apache-2.0 OR MIT | same | Apache-2.0 OR ISC OR MIT; `ring` Apache-2.0 AND ISC | Apache-2.0 OR MIT |
| Transitive footprint (measured) | 23 crates more than today | same | 17 crates more than today | 13 crates more than today with the resolver that was chosen; 18 to 23 with the stock resolver |
| Unsafe and non-Rust code | `snow` forbids unsafe | same | `rustls` forbids unsafe; `ring` contains C and assembly | `snow` forbids unsafe |
| Long-term maintenance | one maintainer; key erasure requested since 2017 | same | `rustls` well staffed, 0.24 and 1.0 planned for late 2026; `ring` without a release since 2025-03 | as A |
| Forward secrecy | yes | yes | yes; resumption and early data disabled | yes |
| Mutual authentication | by Monolith's signatures | same | by TLS CertificateVerify | by Noise, of the transport keys; the card binds them to identities |
| Responder authentication | after the handshake | same | first server flight | second message |
| Initiator identity privacy | hidden from all but a responder that proved the dialed identity | same | same | same, with forward secrecy; Noise rating 8 |
| Responder identity privacy | shown to a prober that knows the identity key | shown to any prober | shown to any prober, with a signature | never transmitted; Noise rating 3 |
| Resistance to active probing | friction: a prober needs the public identity key | none | none: a prober gets the identity key and a fresh signature over a transcript it chose part of | a prober without the card gets no Monolith protocol response, not even a reply, though Tor still shows that the service is reachable; with the card it learns that the key holder is live |
| Transcript binding | signature over h, by Monolith | same | CertificateVerify and Finished, by TLS | Noise handshake hash; every message is authenticated under it |
| Replay resistance | fresh ephemerals in h | same | fresh randoms and key shares | fresh ephemerals; a replayed first message gets a reply nobody can use |
| Downgrade resistance | one suite | same | one version, suite and group configured; negotiation covered by the transcript | one suite; the version is in the prologue |
| Reflection and role confusion | role byte, by Monolith | same | context strings, by TLS | roles are fixed by the pattern |
| Key separation | identity key signs under a Monolith prefix | same | identity key signs cards and, online at every handshake, TLS transcripts | identity key signs cards only and is not needed for a session |
| Key rotation | none needed | none needed | none needed | a new transport key is a new card epoch, announced through the old key and proven before it takes over; no revocation |
| Session resumption | none exists | none exists | exists; disabled | none exists |
| 0-RTT | none exists | none exists | exists; disabled | not with XK |
| Implementation complexity | small | smallest | configuration, two verifiers, a signer | small: fixed messages, card checks |
| Monolith-owned composition | the proof | the proof | no cryptographic construction; the verifiers, the signer and the configuration are Monolith's | five binding rules, listed below |
| Parser before authentication | 192 bytes, fixed | fixed, less | the TLS handshake parser: variable messages and extensions, up to 64 KiB per message | 48 bytes, fixed; then 48 and 235 |
| Fuzzability | glue is small | same | glue is small; `rustls` is fuzzed upstream | glue is small |
| Test vectors | deterministic handshakes through a test hook in `snow` | same | a deterministic handshake needs a test-only provider | deterministic; reproduced by an independent implementation |
| Zeroization | `snow` clears nothing | same | `rustls` clears what it holds; `ring` does not | the resolver clears the keys it holds; `snow` clears nothing of its own (F-R1) |
| Duplicate dependencies | a second `curve25519-dalek` and `sha2`, or a resolver of our own | same | `getrandom` 0.2 next to 0.3 | none in the product; `rand_core` 0.9 next to 0.10 through the property-test crate (F-R1) |
| Tails and Whonix | pure Rust | pure Rust | needs a C compiler; Debian 13 packages older versions | pure Rust |

## Why F and not D

A and B leave the long-term authentication to a construction of Monolith's
own with nothing gained in return. D and F each avoid that, in different
ways, and were the two finalists.

| | D: TLS 1.3 raw public keys | F: Noise XK, key in the card |
| --- | --- | --- |
| Bytes an unauthenticated peer can make the responder parse | a ClientHello with extensions, then encrypted handshake messages; bounded by a cap of ours and by the library | 48, of fixed layout |
| Code reachable before authentication | a general TLS state machine and its parsers | one Diffie-Hellman operation and one tag check |
| What a prober without the card gets | the identity key and a signature | no protocol response |
| What a prober with the card gets | the same, as transferable evidence that the key answered | knowledge that the key holder is live; nothing it can show to others |
| Use of the identity private key | online, one signature per handshake | only when a card is issued |
| Authentication rests on | TLS 1.3, analyzed in many models | Noise XK, analyzed symbolically and computationally, and five rules of ours |
| What Monolith has to get right | verifiers, signer, a dozen configuration choices, checks after the handshake | the five rules |
| Library | well staffed, audited in 2020, three advisories in three years, interface about to change | one maintainer, audited in 2024, one advisory, clears no keys |
| Long-term secrets | one | two |
| Change to Phase 1 formats | none to cards; identity proof and preamble removed; frame overhead 0 | contact card gains a field; identity proof and preamble removed; frames unchanged |
| Measured handshake, both sides, one machine | 0.31 ms; 203, 368 and 193 bytes | 0.44 ms; 48, 48 and 235 bytes |

Reasons for F:

1. The attack surface in front of unauthenticated peers is the smallest of
   any candidate: 48 bytes of fixed layout, one Diffie-Hellman operation,
   one tag. The listener is reachable by anyone who has a contact card.
   Monolith has no update mechanism by design, so a parser error in a
   large dependency stays reachable until the user upgrades by hand.
2. It gives a prober no protocol response. A party that knows only the
   Onion Service address cannot produce a valid first message and gets no
   reply from Monolith. That the service is reachable at the address is
   visible at the Tor level, as for any onion service, and nothing here
   claims otherwise. D hands every caller the identity key and
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

What D had in its favor, and why it was not enough. D has no cryptographic
construction of ours. But a review of the draft specification for D found
one contradiction that would have led an implementer to send the
initiator's identity before verifying the responder, and half a dozen
cases the draft did not cover: a HelloRetryRequest, a responder that sends
no CertificateRequest, the signature algorithm field, KeyUpdate, the
alerts a library sends on its own. None is a flaw of TLS. Each is
something Monolith would have had to specify and test. The glue D needs is
not smaller than the five rules of F, and D's parser, its disclosure to
probers and its use of the identity key at every handshake remain.

## The five rules

Certifying a Diffie-Hellman key with a signing key is the method the Noise
specification names for this situation. It is still composition, and it is
where this design can go wrong.

F1. The card carries the transport key, and the card signature covers it.
    The card is the certificate. No second signed structure is introduced.

F2. The holder of a transport key commits to the identity it acts as, in
    two halves. A, transcript binding: the Noise prologue contains the
    responder's identity key. B, local-state integrity: the local party
    that brings a card and a transport key to a handshake can only be
    made by signing the card from the local identity key
    (`LocalParty::issue`); a card from outside cannot become the local
    one. Neither half proves possession of the identity's private key in
    a session; the card is the identity's authorization of the transport
    key, and Noise proves possession of that key. Without half A there is
    an attack. Mallory signs a card of her own that names Bob's
    endpoint and Bob's transport key. Alice imports it, connects, reaches
    Bob, and Noise succeeds, because Bob does hold that transport key.
    Alice now attributes Bob's messages to Mallory. Nothing in Noise
    prevents this: a certificate says the identity vouches for the key,
    and nothing says the key holder accepts the identity. With the
    identity key in the prologue, Alice hashes Mallory's key and Bob his
    own, and the first message fails. This is the misbinding that the
    SIGMA paper describes for certified Diffie-Hellman keys. Tor's ntor
    handshake mixes the relay identity into its key derivation for the same
    reason. Without half B, local code that paired Bob's transport key
    with Mallory's card would make Bob's side answer and initiate as
    Mallory, which half A cannot see.

F3. The initiator sends its own contact card, without capability, as the
    payload of the third message. The responder checks that the card is
    valid, that the identity is not its own, and that the transport key in
    the card is byte for byte the static key Noise authenticated. Only
    then is the initiator's identity the one in the card. The card is
    inside the handshake, so it is the key holder who presents it.

F4. Successor credentials and rollback. The local side holds, per
    contact, the active card, an authorized successor, a pending successor
    and the retired key (`PROTOCOL.md` section 11.4). A card of another
    key older than the active card or than the authorized successor, a
    card that contradicts the active card or the authorized successor at
    its epoch, a card of the authorized key older than the announced one,
    or a card that states the retired key does not make the session a
    contact session. An older card of the active key does, since its
    holder proves the key that stands for the contact now; the card is not
    taken, so the active card never goes back to a lower epoch. A newer
    card with the active key advances the active card. A newer card with another key becomes the
    contact's credential only through continuity, an announcement in an
    EndpointUpdate on a session of the active key followed by a handshake
    that proves the new key, or through the user's explicit confirmation;
    otherwise it is pending and gives no standing. Promotion retires the
    previous key and withdraws its sessions before the duplicate rule is
    applied. A peer that is not a contact for the session is treated as
    any identity that is not a contact and is not told why. A responder
    applies the rule to the card presented in the third message, an
    initiator to the card it dialed, against the credentials as they are
    when the handshake is complete. This keeps a retired key from being
    used against contacts that promoted its successor, and keeps a copied
    identity key alone from taking a contact over.

    Since Phase 4 one decision applies F4 to every card of a known
    identity, from a handshake in either direction, an announcement, an
    import or a confirmation (`contact::decide` over
    `Credentials::relation`): a card of another key that is not newer than
    the authorized successor is stale from every source, where Phase 3 had
    an import hold it pending. The contact store makes the decision under
    the lock of the contact, keeps the result durable before it is used,
    and keeps the credentials, the retired key included, across restarts.
    On the initiator's side, `read_message_2` returns the authenticated
    responder (`OutboundPeer`), and `OutboundPeer::admit` makes the third
    message, which carries the initiator's card, only for a standing that
    may learn the local identity; for any other there is no third message
    to send.

F5. An X25519 key that is not canonically encoded or is of small order is
    invalid, as a transport key and as an ephemeral key, and a
    Diffie-Hellman result of all zeros ends the handshake. The Noise
    specification leaves this to the application, and `snow` checks
    neither.

Next to these, the card validation enforces key separation where a card
can show a violation: an endpoint that is the identity key, or a transport
key that is the identity key or an endpoint key in Montgomery form.

## The design in detail

Pattern and suite: `Noise_XK_25519_ChaChaPoly_SHA256`.

Prologue, 51 bytes: the 19 ASCII bytes `MONOLITH-SESSION-V1` followed by
the responder's identity public key. The label carries the protocol
version. There is no preamble and no other version field.

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

Every message has a fixed size. A side reads exactly the bytes it expects.
A failure at any point closes the stream without a reply.

What each party learns:

| Party | Learns |
| --- | --- |
| Passive observer of the stream | Three messages of fixed size. Nothing readable. |
| Active party without the responder's card | Nothing. Its first message fails the tag and the responder closes without replying. |
| Active party with the responder's card | That the holder of the transport key is live at the address. No signature and nothing transferable. |
| Party that answers at the address without the transport key | 48 bytes it cannot use. The initiator stops at the second message and has sent no identity. Obtaining the transport key later reveals nothing from what was recorded. |
| Authenticated initiator that is not a contact | That its first message is answered with Close. Nothing about the contact list. |
| Authenticated responder | The initiator's card: identity, transport key, endpoints. |

The last row is not a new disclosure. An initiator dials only identities
it holds as contacts or has asked to become contacts, and both receive its
card anyway.

Outbound pinning. An initiator dials with a card that states the active
key of the contact and accepts nothing else: a responder that cannot answer for that transport
key and that identity key fails message 2, and the initiator reports an
identity mismatch. There is no option to continue and no fallback to an
unknown peer.

Inbound authentication. A responder accepts any initiator with a valid
card. Its handshake does not look at contacts or block lists.
Authorization is a separate step, afterwards (`PROTOCOL.md` sections 6.2,
6.4 and 12). The stale-card rule is part of that step.

Contact card. One field is added to the signed bytes and to the binary
form: the 32-byte transport key. A card grows from 139 to 171 bytes, 187
with a capability. The epoch covers the transport key as it covers the
endpoints.

Changing the transport key. An identity replaces its transport key by
signing a card with a greater epoch, and keeps the old key while it does:
it announces the successor card on sessions of the old key, then opens
sessions with the new key, which each contact promotes in that handshake;
it answers with the new key once its contacts have promoted it
(`PROTOCOL.md` section 11.4). Limitations, accepted for version 1:

- A responder answers with one key at a time (P8). Around the switch a
  contact on the other side of it cannot dial, and the identity reaches it
  from its side.
- If both sides of a contact give up their old keys before the successor
  cards were exchanged, neither reaches the other; the recovery is a card
  handed over out of band and confirmed.
- There is no revocation. Against a contact that promoted the successor,
  the old key is retired. Against a party that has never seen the
  identity, or still holds the old key active, a stolen old key still
  authenticates as the identity.
- A copied identity key alone yields only pending successors. A copied
  identity key with the active transport key yields continuity and is
  indistinguishable from the identity. The identity key cannot be
  replaced.

Invitation capability. Unchanged: inside the ContactRequest, in the
authenticated channel, never on the wire in clear, compared in constant
time, looked at only after the Close has been sent. It is not an identity
or a key and authenticates nobody. It can be checked only after a complete
handshake, which costs a responder three X25519 operations, one key
generation and the validation of one card. The inbound rate limits and
the budget for unknown sessions bound that. No challenge is placed in
front of the handshake, and the Noise pre-shared key is not used.

Transport and replay. Frames are Noise transport messages, as Phase 1
assumes: a 16-byte tag per frame, nonces counting from zero. A frame that
is replayed, dropped, reordered or taken from another session fails
authentication and ends the session. Monolith adds no sequence numbers.
Repetition of messages is handled by the messages: a chat message that is
sent again after a reconnect carries the same MessageId and is delivered
once; a contact card that is sent again is judged by its epoch.

Limits per session, whichever comes first: 24 hours, 2^32 frames in one
direction, 2^40 ciphertext bytes in one direction. The cipher forces none
of them; a Noise cipher state allows 2^64 - 1 messages. They bound the
life of a set of keys. There is no rekey: a side that reaches a limit
sends Close and connects again with a full handshake. There is no
resumption and no data before the handshake is complete.

Key lifecycle:

| Secret | Generated | Lives | Stored | Erased |
| --- | --- | --- | --- | --- |
| Identity private key | by Monolith | until the identity is discarded | vault | by its type; needed only to issue a card |
| Transport private key | by Monolith, independently | until replaced by a new card epoch | vault | by its type; the copy in the Noise state by the resolver, when the handshake ends |
| Noise ephemeral key | by the session crate, OS CSPRNG | one handshake | never | by the resolver, when the handshake ends |
| Chaining key | by Noise | one handshake | never | not by `snow` |
| Frame cipher keys | by Noise | one session | never | by the resolver, when the session ends |
| Invitation capability | by Monolith | until revoked | vault | by its type |

Erasure on drop removes the copies a type controls. It does not reach
copies made by the compiler, memory that was swapped out or crash dumps.

Implementation boundary. One new crate holds everything that touches the
Noise library. A handshake object cannot send or receive frames; a
completed handshake yields an authenticated session, and only that object
can. It owns the session logic of the protocol core. The rest of Monolith
sees plaintext messages on one side and opaque bytes for the stream on the
other.

## Consequences for Phase 1

No Monolith peer has been deployed, so nothing on any network is affected.

- The contact card gains the transport key. Its test vectors, its text
  form and the fuzz seeds that contain cards change. ContactRequest and
  EndpointUpdate grow by 32 bytes; the largest message before
  confirmation is 832 bytes and still fits one padding block.
- The 8-byte preamble, the identity proof message and the feature bits of
  the earlier drafts are removed. Message code 0x0001 becomes unassigned.
- The handshake message sizes become 48, 48 and 235.
- The session logic no longer receives an identity proof. It is told by
  the handshake which identity was authenticated, and gains the
  stale-card standing, and since 2026-10-02 the pending-successor standing
  of F4.
- Unchanged: the frame format and its overhead of 16 bytes, padding, every
  other message, text rules, fingerprints, Ed25519 key validity, contact
  confirmation and duplicate resolution.

## Risks and open points

- F-R1. Library and provider. Closed: `snow` with a resolver of
  Monolith's own. `snow` takes its primitives through four traits
  (Diffie-Hellman, cipher, hash, random source), and there were three
  ways to fill them.

  1. The stock resolver. Its Diffie-Hellman object keeps the private key
     in a plain byte array and never clears it. For Monolith that key is
     the transport key, a long-term secret, and a copy of it would be
     left behind in freed memory by every handshake, in either
     direction. This is finding TOB-SNOW-8 of the 2024 audit and is
     open upstream since 2017. The stock resolver also checks nothing
     about a public key and accepts an all-zero result, so rule F5 would
     have to be enforced around it. It is built on `curve25519-dalek` 4,
     `chacha20poly1305` 0.10 and `sha2` 0.10, next to the versions
     Monolith already uses.
  2. The `ring` resolver. It has no X25519, so it cannot serve this
     suite alone, and it needs a C compiler.
  3. A resolver in the session crate over `x25519-dalek` 3.0.0,
     `chacha20poly1305` 0.11.0 and `sha2` 0.11.0, with `getrandom` as
     the random source. The private key lives in `StaticSecret`, which
     is cleared when dropped. The cipher keys are cleared when dropped.
     The Diffie-Hellman function is where F5 is enforced: it refuses a
     public key that is not canonical or is of small order, and an
     all-zero result, so the rule holds for every key that reaches a
     Diffie-Hellman operation, whichever message it came from.

  The third was chosen, for the erasure of a long-term key and for
  having F5 in one place. That it avoids a second copy of three
  cryptographic crates was not a reason and would not have been enough.

  What it costs: security-sensitive glue that Monolith owns, about 350
  lines with its comments and without its tests. It contains no
  cryptographic construction. Each
  function hands its arguments to one library call: X25519, one AEAD
  seal or open with the Noise nonce layout, SHA-256 update and
  finalize, a read from the operating system source. HMAC and HKDF stay
  those of `snow`. libsignal and rust-libp2p run `snow` in the same
  way.

  Evidence. A prototype of the resolver ran the handshake of
  `PROTOCOL.md` section 16.1 with the fixed keys given there and
  produced the same three messages, the same handshake hash and the
  same first frame as the independent implementation the vectors come
  from, and as `snow` with its stock resolver. The session crate carries
  the same test (`tests::vectors`), so that the build fails if the
  resolver ever differs. A first message whose ephemeral key is a point
  of small order was refused in the Diffie-Hellman step.

  What it does not solve. `snow` itself keeps the chaining key in its
  handshake state and copies of derived keys on its stack, and clears
  neither. The resolver cannot reach them. Section "Key lifecycle" and
  `CRYPTOGRAPHY.md` section 8 say so.

  Dependencies. `x25519-dalek` 3.0.0 depends on `rand_core` 0.10. The
  property-test crate, which is used for tests only, depends on
  `rand_core` 0.9. `rand_core` is a crate of traits and the older one
  is in test builds only. The duplicate check of `cargo deny` does not
  count crates that are in the tree only through development
  dependencies, and `deny.toml` says so in so many words. No
  cryptographic crate is in the tree twice, and a second version of any
  crate in a product build still fails the check.
- F-R2. The five rules have no external review. This is the first thing
  to put in front of a cryptographer if one becomes available. The scope:
  F1, identity to transport certification; F2, transcript and local-party
  binding; F3, the initiator's card bound to its static key; F4,
  successor, rollback and promotion; F5, X25519 validity; and the
  rotation and retirement state machine. The internal reviews, that of
  2026-10-02 included, are not an external review, and nothing here is
  formally verified.
- F-R3. `snow` has one maintainer and no release since July 2025. The
  Noise handshake is small and specified; replacing the library is
  contained in one crate.
- F-R4. No revocation of a transport key. Rotation retires a key at each
  contact that promotes the successor, and nowhere else.
- F-R5. A stale card presented to a responder that never saw a newer one
  is accepted. Same root as F-R4. Since Phase 4 a responder that saw a
  newer one keeps it across restarts, in the vault.
- F-R6. A session is not bound to the onion address that was dialed
  (`PROTOCOL.md` open question P2).

`native-tls` and `openssl-sys` stay banned. Nothing here needs them.

## Public keys are not secrets

Knowing a responder's public keys is not possession of a secret and is
never access control, authorization or authentication of the initiator. A
caller needs the responder's identity key and transport key to get any
reply. Both are in every contact card. What the design provides is that a
party with the onion address alone cannot produce a valid first message
and gets no Monolith protocol response, and that nobody is handed a
signature. It does not hide that the Onion Service is reachable; Tor
shows that to anyone with the address. It keeps nobody out who holds a card.

If Monolith ever requires a real secret before it answers, that secret has
to be a secret. The invitation capability is not one in general: the user
may publish the card that carries it (PROTOCOL.md section 12.2).

## Questions closed by this record

- Q1: Noise XK with a card-certified transport key.
- Q2: the responder's identity is not disclosed by the handshake.
- Q3: the prologue is used for binding, not as a precondition.
- Q4: no Monolith proof input exists.
- Q5: XK.
- Q6: the certificate is the contact card; its lifetime is its epoch.
- Q7: TLS with raw public keys works with `rustls`; not used.
- Q8: the onion key is not bound; `PROTOCOL.md` P2 stays open.
- Q9: limits as above; reconnect, no rekey.
- Q10: `snow` with a resolver over the current crates, F-R1.
- Q11: the responder first, by the pattern.

## Sources

Accessed 2026-10-01. Measurements were made with throwaway prototypes that
are not part of the repository.

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
- TLS 1.3: https://www.rfc-editor.org/rfc/rfc9846 , which obsoletes
  https://www.rfc-editor.org/rfc/rfc8446
- Raw public keys: https://www.rfc-editor.org/rfc/rfc7250
- rustls: https://docs.rs/rustls/0.23.45 ,
  https://github.com/rustls/rustls/blob/main/audit/TLS-01-report.pdf ,
  https://rustsec.org/packages/rustls.html
- iroh: https://docs.rs/crate/iroh/1.3.0/source/src/tls.rs
