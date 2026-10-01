# ADR 0002: Session protocol

Status: decided. The session layer is TLS 1.3 with mutually authenticated
raw public keys. Implementation has not started and waits for review of
this record.
Date: 2026-10-01

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

Phase 0 proposed Noise XX with per-connection static keys and a signed
identity proof. The review of Phase 0 kept it as a candidate and asked for
a comparison that does not assume it wins. This record is that comparison
and its outcome.

## Candidates

### A. Noise XX, per-connection static keys, signature over the handshake hash

`Noise_XX_25519_ChaChaPoly_SHA256`. Each side generates a new static key
for every connection. After the handshake each side sends, inside the
channel, an Ed25519 signature by its identity key over the handshake hash
and a few other fields (the AuthProof of the earlier drafts).

What Noise provides here: a forward-secret channel between the holders of
two single-use static keys that stand for nothing. What Monolith provides:
all long-term authentication, through a signature construction of its own.
The Noise specification mentions this use in one sentence (section 11.2,
channel binding) and does not analyze it. Structurally it is SIGMA: sign
the transcript, and let the AEAD under the derived key play the part of the
MAC over the signer's identity.

The static keys of XX are used for a side effect: message 3 shows the
responder that the initiator computed the same handshake hash, prologue
included, so an initiator has to know the responder's identity key to get
a proof. That is friction for a prober, not access control (see "Public
keys are not secrets").

A variant, A2, is what libp2p specifies: sign the Noise static public key
with the identity key and send the signature in the handshake payload.
go-libp2p uses a new static key per handshake. This is the most widely
deployed construction of the family. Its signature covers nothing but the
static key: no session, no time, no peer. Whoever obtains one static
private key together with the public signature holds a credential for the
identity that never expires. With per-connection keys that requires
reading the memory of a running session, and `snow` does not erase keys.

### B. Noise NN and signatures over the handshake hash

`Noise_NN_25519_ChaChaPoly_SHA256`, then the same signatures as in A.
Noise provides an unauthenticated ephemeral exchange and nothing else, and
the pattern claims nothing it does not deliver. This describes the security
model more honestly than A. It is still Monolith's own composition, and it
rebuilds by hand the structure that TLS 1.3 standardizes: ephemeral
exchange, signature over the transcript with a role-specific context, key
confirmation. NN has no third handshake message, so a responder cannot see
that the initiator knows its identity before it answers; the responder
proves its identity to any party that connects, as in D.

### C. Noise with a persistent X25519 static key bound to the identity

Each identity holds a long-lived X25519 key, signed by the Ed25519
identity. C1 carries the signature in an XX payload (libp2p with a stored
key). C2 publishes the static key in the contact card and uses XK, which
gives the strongest identity hiding of all options: the responder's static
key is never transmitted and the initiator's is sent to an authenticated
responder.

Costs. A second long-term secret to store, back up and rotate. A signature
over a static key is a standing credential, so C needs expiry or
revocation rules that no deployed specification provides and Monolith
would have to design. C2 changes the contact card, which is a Phase 1 wire
format with published test vectors, and couples card distribution to
transport key lifetime. The X25519 key must not be derived from the
Ed25519 key for convenience; kept separate, it is one more thing a user
has to carry between devices and one more key whose theft impersonates the
identity.

### D. TLS 1.3 with raw public keys on both sides

TLS 1.3 (RFC 8446) with RFC 7250 raw public keys. The Ed25519 identity key
is the TLS authentication key of each side. The initiator accepts exactly
one key, the identity it dialed. The responder accepts any valid key and
leaves the question of who that is to the layers above.

What TLS provides: everything cryptographic. Each side's CertificateVerify
is a signature by its identity key over the handshake transcript with a
role-specific context string; Finished confirms the keys; the client sends
its key and signature only after it has verified the server. What Monolith
provides: two decisions (is this the key I dialed; is this key well
formed) and a configuration that switches off what is not needed.

### E. Other constructions

Other maintained Noise implementations in Rust (`clatter` 2.3.0,
`noise-protocol` 0.2.1 with `noise-rust-crypto` 0.6.2) have no audit and no
known deployment of note, and they depend on the same older generation of
cryptographic crates as `snow`. They change the library under A to C, not
the comparison. No other session stack was found that is mature, reviewed
and a better fit than the four above. Experimental frameworks were not
searched further.

## Comparison

The table states what was found, with sources at the end. "Composition"
means security-critical protocol logic that Monolith would specify itself
and that no standard or published analysis covers.

| | A: XX + signature over h | B: NN + signature over h | C: persistent certified static key | D: TLS 1.3 raw public keys |
| --- | --- | --- | --- | --- |
| Cryptographic maturity | X25519, ChaCha20-Poly1305, SHA-256, Ed25519 | same | same | same primitives, in the TLS 1.3 key schedule (HKDF-SHA256) |
| Protocol maturity | Noise revision 34 (2018), status "official/unstable"; the signature step is not part of it | same | Noise as designed; the certificate is outside the specification | RFC 8446 (2018) and RFC 7250 (2014), IETF standards |
| Implementation maturity | `snow` 0.10.0 (2025-07); single owner, "reasonable-effort" maintenance, no release since | same | same | `rustls` 0.23.45 (2026-09); a team, frequent releases; 0.24 and 1.0 planned for late 2026 |
| Audit history | Trail of Bits, January 2024, of an earlier `snow` commit: ten findings, eight fixed, two open (one of them: keys are not cleared). The proof construction: none | same | same; the certificate rules: none | Cure53, 2020, of `rustls` with `ring` and `webpki`: four low or informational findings. Nothing later was found |
| Known deployments | `snow`: libp2p, libsignal (attestation), 1Password. A signature over h: one deployment found | same | static keys with Noise: Lightning, I2P, WireGuard; certified static: libp2p, Nebula | TLS 1.3 everywhere; raw public keys with `rustls`: iroh, over QUIC, with an Ed25519 signer of its own and a server that accepts any client key |
| MSRV | 1.85 | 1.85 | 1.85 | `rustls` 1.71, `ring` 1.66 |
| Licence | Apache-2.0 OR MIT | same | same | `rustls` Apache-2.0 OR ISC OR MIT; `ring` Apache-2.0 AND ISC |
| Transitive footprint (measured) | 23 crates more than today | same | same | 17 crates more than today |
| Unsafe and non-Rust code | `snow` forbids unsafe; its crypto crates contain some | same | same | `rustls` forbids unsafe; `ring` contains C and assembly from the BoringSSL lineage |
| Long-term maintenance | one maintainer; open requests for key erasure since 2017 | same | same | `rustls` well staffed; `ring` has had no release since 2025-03, its author stepped back in 2025 and the `rustls` team can publish fixes; the provider is replaceable |
| Forward secrecy | yes | yes | yes | yes; resumption and early data disabled |
| Mutual authentication | by Monolith's signatures | by Monolith's signatures | by Noise, through a certificate | by TLS CertificateVerify on both sides |
| Responder authentication | after the handshake, first frame | same | in the handshake | in the handshake, first server flight |
| Initiator identity privacy | hidden from all but a responder that proved the dialed identity | same | C2: same, from Noise itself | same, from TLS itself (RFC 8446 appendix E.1) |
| Resistance to active probing | a prober needs the responder's public identity key to be handed a proof; with it, it gets one | none: any prober gets the proof | C1 none; C2 a prober needs the public static key | none: any prober that speaks the profile learns the responder's identity key and gets a fresh signature |
| Transcript binding | signature over h, by Monolith | same | Noise handshake hash; the certificate is not bound to the session | CertificateVerify and Finished over the transcript hash, by TLS |
| Replay resistance | fresh ephemerals in h | same | certificate replayable by design; handshake fresh | fresh randoms and key shares in the transcript |
| Downgrade resistance | one suite, no negotiation | same | same | one version, one suite, one group configured; negotiation is covered by the transcript |
| Reflection resistance | role byte in the signed input, by Monolith | same | Noise roles | distinct context strings for client and server, by TLS |
| Role confusion resistance | by Monolith's signed input | same | by Noise | by TLS |
| Key separation | identity key signs under a Monolith prefix only | same | a second long-term key, kept separate | identity key signs contact cards and TLS CertificateVerify; the two inputs cannot collide |
| Key rotation | identity independent of transport keys | same | static key rotation rules to be designed | identity independent of transport keys |
| Session resumption | none exists | none exists | none exists | exists in TLS; disabled on both sides |
| 0-RTT | none exists | none exists | IK has it; not used | exists in TLS; disabled, and impossible without resumption |
| Implementation complexity | small; proof encoding, order rule, state handling | smallest | largest of the Noise options | configuration and two verifiers; no protocol logic |
| Monolith-owned composition | one construction: the proof | one; two with a confirmation step | one, plus credential lifetime rules | none |
| Parser complexity before authentication | three fixed-size records, 192 bytes | two fixed-size records | fixed-size if the certificate is | the TLS handshake parser: variable-size messages and extensions, bounded by the library at 64 KiB per message |
| Fuzzability | glue is small; `snow` has fuzz targets | same | same | glue is small; `rustls` is fuzzed upstream continuously |
| Test vectors | Noise vectors exist; deterministic handshakes possible through an ungated test hook | same | same | RFC 8448 traces for TLS; a deterministic handshake needs a test-only provider |
| Zeroization | `snow` clears nothing; open audit finding | same | same, and a long-term key lives in it | `rustls` clears traffic secrets and keys on drop; `ring` does not clear its own state |
| Duplicate dependencies | a second `curve25519-dalek` (4 next to 5), `sha2`, `digest`; or a resolver written by Monolith | same | same | none among cryptographic crates; `getrandom` 0.2 next to 0.3, which today is a test dependency |
| Tails and Whonix | pure Rust | pure Rust | pure Rust | needs a C compiler to build; Debian 13 packages `rustls` 0.23.26 and `ring` 0.17.14 |

## Decision

Monolith uses TLS 1.3 with raw public keys on both sides (candidate D).

- Protocol: TLS 1.3, RFC 8446. Certificates: raw public keys, RFC 7250, in
  both directions.
- Library: `rustls` 0.23.45, pinned exactly, with the features `ring` and
  `std` and nothing else. TLS 1.2 is not compiled in.
- Crypto provider: `ring` 0.17.14, restricted to the algorithms below.
- Suite: `TLS_CHACHA20_POLY1305_SHA256` (0x1303). Group: x25519 (0x001D).
  Signature scheme: ed25519 (0x0807). There is no second choice for any of
  them, so there is nothing to negotiate.
- Identity binding: the TLS end-entity key of each side is its Monolith
  identity key, as an Ed25519 SubjectPublicKeyInfo. No certificate, no
  second key, no Monolith-defined proof.
- Signatures with the identity key are made and verified by
  `ed25519-dalek`, through the signing and verification interfaces of
  `rustls`. The private key is not handed to the provider, and
  verification is strict, as everywhere else in Monolith.
- Application protocol: ALPN identifier `monolith/1`.

`PROTOCOL.md` sections 3, 4 and 6 give the profile at the level of bytes.
`CRYPTOGRAPHY.md` gives the properties and the key lifecycle.

### Why D

1. It leaves Monolith no cryptographic composition to get wrong. In A and
   B the step that authenticates the long-term identity is Monolith's own.
   It is small and it follows a known shape, but nobody has analyzed it,
   and this project has no cryptographer to review it. In D the same step
   is TLS 1.3's CertificateVerify, which has been analyzed in several
   formal models and attacked in deployment for years.
2. The properties Monolith needs are stated by the standard and not derived
   by Monolith: forward secrecy, mutual authentication, binding of each
   signature to the transcript and to the role, downgrade protection, and
   protection of the client's identity against active attackers.
3. The privacy order comes from the protocol. A TLS client sends its key
   only after it has verified the server's key, signature and Finished. The
   prototype confirms it for `rustls`: with a server key other than the
   pinned one, the client sends an alert and never sends its own key.
4. The implementation is the better maintained one, by a wide margin, and
   it erases the secrets it holds.
5. Another implementer needs no Monolith-specific cryptography. Any TLS 1.3
   stack with raw public key support can speak the profile.

### What D costs

- A larger parser in front of unauthenticated peers. Noise reads 192 bytes
  in three fixed records. TLS parses a ClientHello with extensions and
  accepts handshake messages of up to 64 KiB. The parser is memory-safe
  Rust and is fuzzed upstream, and Monolith adds a cap of its own (see
  below), but the surface is larger and that is a real cost.
- A provider that contains C and assembly, and needs a C compiler to build.
- More to switch off: TLS 1.2, resumption, tickets, early data, server name
  indication, certificate compression. Each is disabled by configuration
  or by not compiling it, and each has a test.
- A handshake that shows the peer which TLS library and version is in use.
  Only the peer sees it; the stream is inside Tor.
- No fixed-size handshake records and no deterministic handshake with the
  stock provider.
- Two cryptographic libraries in the process: `ed25519-dalek` and `sha2`
  for identities, `ring` for the session. The rule "one implementation of
  each primitive" in `DEPENDENCIES.md` is relaxed to say so.
- The probing friction of A is gone. See the next section.
- Changes to provisional parts of the Phase 1 protocol. See "Consequences
  for Phase 1".

### Why not the others

- A. The static keys carry no meaning and exist for the prologue effect,
  which is friction and not protection. The authentication is Monolith's
  composition. `snow` clears no key material, has one maintainer, and
  either duplicates the curve implementation or needs a resolver written
  by Monolith, which is more security-sensitive code of our own.
- A2. A signature over a static key is a reusable credential. That is
  acceptable for libp2p's threat model and not attractive here, least of
  all on a library that leaves keys in freed memory.
- B. Honest about what Noise provides, but it is TLS 1.3's authentication
  written again by hand, without TLS 1.3's analysis, and with the same
  disclosure to probers as D.
- C. A second long-term secret and a lifetime policy for a credential,
  both of which Monolith would have to design. C2 would also change the
  contact card.

Dependency duplication did not decide this. Had a Noise option been the
better construction, the duplicate would have been accepted and
documented.

## Public keys are not secrets

In D any party that can reach the listener and speak the profile learns
the responder's identity public key and receives a signature that proves
the identity is live at that address. Candidate A made that depend on
knowing the identity key beforehand.

That key is public data. It is in every contact card, next to the onion
address, and the two travel together. Knowing it is not possession of a
secret and was never access control, authorization or authentication of
the initiator. What A offered was that a party holding only the onion
address was not handed a proof. Onion addresses of version 3 services are
not enumerable, so that party is rare. The property is given up, knowingly.

What a prober cannot learn, in any candidate: whether an identity it
presents is a contact, blocked, declined or unknown (`PROTOCOL.md`
section 12.1).

If Monolith ever requires a real secret before it does anything
identifying, that secret has to be a secret. TLS has a place for one, an
external pre-shared key. It is not used in version 1: `rustls` 0.23 has no
interface for it, and inventing a challenge in front of the handshake is
exactly the kind of construction this record avoids.

## Authentication

There is no AuthProof. The authentication messages are those of TLS 1.3.

    Initiator (TLS client)                      Responder (TLS server)

      ClientHello
        x25519 key share, suite 0x1303,
        ALPN monolith/1, raw public key
        for both certificate types      ---->
                                                ServerHello (key share)
                                                {EncryptedExtensions}
                                                {CertificateRequest}
                                                {Certificate: responder
                                                   identity key}
                                                {CertificateVerify}
                                        <----   {Finished}
      checks: the key is the identity
        that was dialed; the signature;
        Finished

      {Certificate: initiator identity key}
      {CertificateVerify}
      {Finished}                        ---->
                                                checks: the key is a valid
                                                  key and not its own; the
                                                  signature; Finished

      [frames]                          <--->   [frames]

Braces mark messages encrypted under handshake keys, brackets under
application keys.

Each CertificateVerify is an Ed25519 signature over these 130 bytes, as
RFC 8446 section 4.4.3 defines them:

    64 bytes   0x20 repeated
    33 bytes   "TLS 1.3, server CertificateVerify" for the responder,
               "TLS 1.3, client CertificateVerify" for the initiator
     1 byte    0x00
    32 bytes   SHA-256 transcript hash of the handshake up to and
               including the signer's Certificate message

The transcript covers both hello messages, so it binds the protocol
version, the suite, the group, both key shares, both random values, the
ALPN identifier and the certificate types. The context string binds the
role. The responder's transcript ends before the initiator's key appears;
the initiator's includes the responder's key and signature.

Order. The responder proves its identity first. The initiator reveals its
identity only after it has verified that proof against the identity it
dialed. This resolves the order that Phase 1 left open.

What each party learns:

| Party | Learns |
| --- | --- |
| Passive observer of the stream | That TLS 1.3 is spoken and that the ALPN identifier is `monolith/1`. No identity. Inside Tor, only the two endpoints see the stream at all. |
| Active party that connects and speaks the profile, without authenticating | The responder's identity key and a signature showing that it is live at this address. |
| Active party that answers at the address without the identity key | That someone connected, and what the ClientHello shows of the library. Not the initiator's identity: the initiator stops when the proof fails. |
| Authenticated initiator the responder does not hold as a contact | The same as the prober above, and that its own first message is answered with Close. Nothing about the contact list. |
| Authenticated responder | The initiator's identity key. |

Outbound pinning. The initiator's verifier accepts one key: the identity
passed to the dial. Anything else ends the handshake with
`IdentityMismatch`. There is no option to accept another key and no
fallback to an unknown peer.

Inbound authentication. The responder's verifier accepts any key that is a
valid Monolith key and not its own. It must not look at the contact list,
the block list or any other record of the identity: a handshake that
succeeded or failed depending on standing would be an oracle. What the
identity is allowed to do is decided afterwards, by the contact
confirmation of `PROTOCOL.md` section 6.4.

Checks that are Monolith's, on both sides:

1. The certificate is a raw public key and there is exactly one.
2. It is the 44-byte SubjectPublicKeyInfo of an Ed25519 key, byte for
   byte: the 12 bytes `30 2a 30 05 06 03 2b 65 70 03 21 00` and the key.
3. The key is valid under `PROTOCOL.md` section 10.1.
4. The key is not the local identity key.
5. Initiator only: the key is the one that was dialed.
6. The CertificateVerify signature verifies under strict Ed25519 rules.
7. After the handshake: the version is TLS 1.3, the suite is 0x1303, and
   the negotiated ALPN identifier is `monolith/1`. A handshake that
   completed without ALPN is rejected.

Every rejection by these checks ends the handshake in the same way.

## Invitation capability

The capability stays where Phase 1 put it: in the ContactRequest, inside
the authenticated channel. It is never on the wire in clear, it is
compared in constant time, and it is looked at only after the Close has
been sent. It is not an identity, a key or a transport secret, and it does
not authenticate anyone.

It can be checked only after a complete handshake. An attacker with a
contact card can therefore make a responder perform one key generation,
one X25519 operation, one signature and one verification per attempt, and
hold one session slot until the first message or the timeout. The inbound
rate limits and the budget for unknown sessions bound this
(`RESOURCE_LIMITS.md`). No challenge is placed in front of the handshake.

## Transport, replay and limits

Frames. Monolith frames (`PROTOCOL.md` section 5) are written into the TLS
stream as application data, with the frame overhead parameter T = 0: the
layer below protects them. A frame may span several TLS records. Padding
is unchanged.

Replay and order. TLS numbers every record and authenticates the number. A
record that is replayed, dropped, reordered or taken from another session
fails authentication and ends the session. Monolith adds no sequence
numbers of its own. Application-level repetition is a different thing and
is handled where it arises: a chat message that is sent again after a
reconnect carries the same MessageId and is delivered once; a contact card
that is sent again is judged by its epoch.

Rekeying. Monolith never rekeys a session. When a limit is reached it sends
Close and connects again with a full handshake. A KeyUpdate from the peer
is legal TLS and is handled by the library.

Limits per session, whichever comes first:

| Limit | Value |
| --- | --- |
| Age | 24 hours |
| Frames sent in one direction | 2^32 |
| Plaintext bytes sent in one direction | 2^40 |

None of these is forced by the cipher. TLS 1.3 with ChaCha20-Poly1305 has
no confidentiality limit below the 2^64 record sequence space (RFC 8446
section 5.5), and a single failed record ends the session, so an attacker
gets one forgery attempt. The limits exist so that a session key has a
bounded life and a long-running session regularly gets new ephemeral keys.
The age limit is the one that is reached in practice. Ciphertext is at
most 22 bytes more per TLS record than plaintext.

Resumption and early data are disabled on both sides: the responder issues
no tickets and keeps no session store, the initiator neither offers nor
stores sessions. Every connection is a full handshake.

Handshake bounds. A side stops and closes if it has received more than
4096 bytes of TLS data and the handshake is not complete. The flights of
this profile are 203, 368 and 193 bytes with `rustls` 0.23.45. The
handshake timeout of `RESOURCE_LIMITS.md` applies as before.

## Key lifecycle

| Secret | Generated | Lives | Stored | Erased |
| --- | --- | --- | --- | --- |
| Identity private key | by Monolith, OS CSPRNG | until the identity is discarded | vault, or memory in ephemeral mode | by Monolith's key type on drop |
| TLS ephemeral X25519 key | by `ring`, OS CSPRNG | one handshake | never | not explicitly; `ring` does not clear |
| Handshake and traffic secrets | TLS key schedule | one session | never | by `rustls` on drop |
| Record keys inside the provider | from the traffic secrets | one session | never | not explicitly; `ring` does not clear |
| Invitation capability | by Monolith, OS CSPRNG | until revoked | vault | by its type on drop |
| Signature buffers | per handshake | one handshake | never | not secret |

Erasure on drop removes the copies a type controls. It does not reach
copies made by the compiler, memory that was swapped out, crash dumps, or
state inside the provider. This is stated as a limit, not hidden.

## Consequences for Phase 1

The parts of `PROTOCOL.md` that were marked provisional are replaced. No
Monolith peer has been deployed, so nothing on any network is affected.

- The 8-byte preamble is removed. Version selection is the ALPN
  identifier, which the handshake authenticates.
- The three Noise handshake records are removed.
- AuthProof is removed, and with it the feature bits. Message code 0x0001
  becomes unassigned. A later extension is selected by a new ALPN
  identifier.
- The frame overhead T becomes 0. The value of the frame length field is
  16 less than before; plaintext sizes, the largest body and the chunk
  size do not change.
- The session states keep their names. `IdentityAuth` no longer waits for
  a message; it is the point at which the checks listed above are applied
  to a completed handshake.
- The session logic no longer receives an AuthProof. It is told by the
  handshake which identity was authenticated.

Unchanged: contact cards, fingerprints, key validity, every other message,
text rules, frame plaintext layout and padding, contact confirmation, and
the published test vectors of `PROTOCOL.md` section 16. The fuzz seeds for
frames and messages are regenerated because the message list and the frame
overhead change.

## Implementation boundary

One new crate holds everything that touches the TLS library. Nothing else
in Monolith calls it.

- `Initiator` and `Responder` drive a handshake from bytes in to bytes out,
  without sockets. Neither can send or receive a frame.
- A completed handshake yields an `AuthenticatedSession` and the identity
  that was proven. Only that type can send and receive frames, and it
  applies the Phase 1 session logic to every message.
- The rest of Monolith sees plaintext messages on one side and opaque bytes
  for the stream on the other. Replacing the TLS library or the provider
  does not touch contact logic.

## Risks and open points

- R1. The pre-authentication parser is that of a general TLS library. An
  error in it is reachable by anyone who knows the onion address. The
  byte cap, the timeout and upstream fuzzing reduce the exposure; they do
  not remove it.
- R2. `ring` has had no release since March 2025 and its author stepped
  back that year; the `rustls` team can publish fixes. If fixes stop, the
  provider has to change. `aws-lc-rs` is the alternative and needs NASM
  or prebuilt objects on Windows.
- R3. `rustls` 0.24 and 1.0 are planned for late 2026 and change the raw
  public key interface. The pin will need a planned migration. Version
  0.23.45 is the first without RUSTSEC-2026-0285; nothing older is
  acceptable, which rules out the version Debian 13 ships.
- R4. The last public audit of `rustls` is from 2020. Monolith's verifiers
  and configuration are not audited at all.
- R5. A deterministic handshake for test vectors needs a provider written
  for tests. Whether that is worth it is decided during implementation.
- R6. The responder shows its identity key and a fresh signature to any
  prober. Accepted above; listed because it is the visible difference to
  the Phase 0 proposal.
- R7. Per-handshake memory has not been measured. It is bounded by the
  byte cap and the library's fixed buffers; the number goes into
  `RESOURCE_LIMITS.md` when the code exists.

For external review, if one becomes possible: the two verifiers and the
checks after the handshake; the configuration that disables resumption,
tickets and early data; and the decision to use the identity key directly
as the TLS key.

## Questions closed by this record

- Q1: D.
- Q2: disclosure of the responder's key to a connecting party is accepted.
- Q3, Q5: no prologue; Noise is not used.
- Q4: no Monolith proof input exists.
- Q6: no certificate exists.
- Q7: answered by the prototype: mutual raw public keys work; a responder
  can accept any key; no server name is sent; resumption and tickets can
  be switched off; an exporter is available.
- Q8: the onion service key is not bound into the handshake. Open question
  P2 of `PROTOCOL.md` stays as it is.
- Q9: limits as above; reconnect, no rekey.
- Q10: `ring`; `snow` is not used.
- Q11: the responder proves first, by TLS.

`native-tls` and `openssl-sys` stay banned. Nothing here needs either.

## Sources

Accessed 2026-10-01. Measurements were made with throwaway prototypes that
are not part of the repository.

- TLS 1.3: https://www.rfc-editor.org/rfc/rfc8446 (4.4.3 CertificateVerify,
  5.5 limits, appendix E.1 identity protection)
- Raw public keys: https://www.rfc-editor.org/rfc/rfc7250
- Ed25519 in SubjectPublicKeyInfo: https://www.rfc-editor.org/rfc/rfc8410
- rustls: https://docs.rs/rustls/0.23.45 ,
  https://github.com/rustls/rustls/releases ,
  https://github.com/rustls/rustls/issues/2400 (0.24 and 1.0)
- rustls raw public key example:
  https://github.com/rustls/rustls/blob/v/0.23.45/openssl-tests/src/raw_key_openssl_interop.rs
- rustls audit: https://github.com/rustls/rustls/blob/main/audit/TLS-01-report.pdf
- rustls advisories: https://rustsec.org/packages/rustls.html
- ring: https://github.com/briansmith/ring/discussions/2414 ,
  https://rustsec.org/advisories/RUSTSEC-2025-0007.html
- iroh: https://docs.rs/crate/iroh/1.3.0/source/src/tls.rs
- Noise specification, revision 34: https://noiseprotocol.org/noise.html
- Noise Explorer: https://eprint.iacr.org/2018/766
- fACCE analysis of Noise: https://eprint.iacr.org/2019/436
- A Spectral Analysis of Noise:
  https://www.usenix.org/conference/usenixsecurity20/presentation/girol
- SIGMA: https://www.iacr.org/archive/crypto2003/27290399/27290399.pdf
- snow: https://github.com/mcginty/snow , https://docs.rs/snow/0.10.0 ,
  https://rustsec.org/advisories/RUSTSEC-2024-0011.html
- snow audit:
  https://github.com/trailofbits/publications/blob/master/reviews/2024-03-agilebits-snow-securityreview.pdf
- libp2p Noise: https://github.com/libp2p/specs/blob/master/noise/README.md
- libp2p TLS: https://github.com/libp2p/specs/blob/master/tls/tls.md
- clatter: https://github.com/jmlepisto/clatter
- noise-rust: https://github.com/blckngm/noise-rust
- Lightning transport: https://github.com/lightning/bolts/blob/master/08-transport.md
- I2P NTCP2: https://i2p.net/en/docs/specs/ntcp2
