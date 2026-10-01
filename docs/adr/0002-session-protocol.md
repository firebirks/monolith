# ADR 0002: Session protocol

Status: provisional. No option is selected. A decision is required before
Phase 2 starts, and nothing in Phase 1 depends on it.
Date: 2026-10-01

## Context

Two peers connected over a Tor stream need mutual authentication of their
Monolith identities, a confidential channel with forward secrecy, and
resistance to replay and relay. Tor already encrypts Onion Service traffic
end to end. The session layer is defense in depth, and it is what ties a
connection to an identity that is independent of the onion address.

Fixed points, whatever construction is chosen:

- nothing home-made: no custom key exchange, cipher or key schedule;
- the least possible novel security-critical composition;
- the Monolith identity is an Ed25519 key that is distinct from the Onion
  Service key (ADR 0001);
- an initiator does not reveal its identity to an endpoint that has not
  proved the identity it dialed;
- bounded parsing in front of unauthenticated peers;
- nothing sent to a peer that describes the platform or the build.

Phase 0 proposed Noise XX with per-connection static keys and a signed
identity proof. Review of Phase 0 found that proposal acceptable as a
candidate but not ready to be frozen, for three reasons:

1. The persistent identity is not the Noise static identity. Long-term
   authentication comes from a signature layered on top of Noise. That
   layer is a protocol composition owned by Monolith and has to be judged
   as one.
2. The Phase 0 text described the prologue binding as if it restricted who
   can talk to a responder. It does not. See "The prologue is not access
   control" below.
3. TLS 1.3 was set aside partly because its crypto providers contain C and
   assembly. That is not a security argument by itself.

## Criteria

Each option is described under the same eleven headings:

1. authentication semantics
2. forward secrecy
3. identity privacy
4. probing behavior
5. number of custom protocol constructions
6. dependency maturity
7. implementation complexity
8. parsing surface before authentication
9. auditability
10. key storage implications
11. compatibility with identity rotation and endpoint migration

Implementation language is not a criterion. A cryptographic library is
judged on audit history, deployment history, cryptographic review, exposure
of memory-unsafe code to hostile input, parser complexity, dependency
surface, maintenance, published advisories, and how much Monolith-specific
composition it leaves to be written.

## Option A: Noise XX, per-connection static keys, transcript signature

`Noise_XX_25519_ChaChaPoly_SHA256`. Each side generates a new static key for
every connection. After the handshake each side sends an Ed25519 signature
over the handshake hash inside the channel. This is the construction
written up in `CRYPTOGRAPHY.md`.

1. Authentication. The Noise handshake authenticates only a single-use
   static key that stands for nothing. All long-term authentication comes
   from the Ed25519 signature over the handshake hash, which the Noise
   specification describes as channel binding (section 11.2) and which has
   the shape of SIGMA. The static-key tokens of XX are used for a side
   effect: message 3 shows the responder that the initiator computed the
   same handshake hash, prologue included.
2. Forward secrecy. Yes. Every Diffie-Hellman key is single-use.
3. Identity privacy. Nothing is visible to a passive observer. The
   responder proves first, to whoever completed the handshake. The
   initiator proves only after it has verified the responder.
4. Probing. Completing the handshake requires the responder's identity
   public key in the prologue. A party that has it receives a fresh
   signature showing that the identity is live at that address. A party
   that holds candidate keys can test them against message 2.
5. Custom constructions. One: the identity proof (its signed input and the
   order rule). In addition it uses two standard features in a non-standard
   role: XX static keys that carry no identity, and the prologue as a
   precondition.
6. Dependencies. `snow` 0.10: the usual Noise implementation in Rust and
   the back end of `libp2p-noise`; no formal audit; does not zeroize key
   material; slow release cadence; one advisory, fixed
   (RUSTSEC-2024-0011). Its crypto back end is selectable and not chosen.
7. Complexity. Low. Three fixed-size records and two fixed-size frames.
8. Parsing before authentication. 8 bytes, then 32, 96 and 64 byte records,
   then one 1040-byte frame with a 104-byte body.
9. Auditability. The specification is short. Noise XX has been analyzed
   formally, but those results are about static keys that identify a party.
   They say nothing about this proof. A reviewer has to reason about the
   composition from scratch and about why the static keys are there at all.
10. Key storage. None beyond the identity key.
11. Rotation. The identity is independent of transport keys and endpoints.

## Option B: Noise NN and transcript signatures

`Noise_NN_25519_ChaChaPoly_SHA256`, then the same signed proofs as in A.

1. Authentication. Noise provides an unauthenticated ephemeral key exchange
   and nothing else. Authentication is entirely the transcript signature,
   sent under the derived keys. This is SIGMA written with Noise, and the
   handshake claims nothing it does not deliver: there are no static keys
   that could be mistaken for identities.
2. Forward secrecy. Yes.
3. Identity privacy. Same ordering choice as A. But NN has no handshake
   message from the initiator after the responder's ephemeral key, and
   transport keys do not depend on the prologue, so a responder cannot see
   prologue agreement before it sends its proof. Responder-first then shows
   the responder's identity to any party that connects. Initiator-first
   would show the initiator's identity to whoever answers at the address,
   including someone who took over the Onion Service key.
4. Probing. With responder-first, any connecting party obtains the proof.
   Getting A's behavior back needs an extra confirmation step from the
   initiator, which is a second custom construction.
5. Custom constructions. One, or two with a confirmation step.
6. Dependencies. As A.
7. Complexity. Lowest. Two fixed-size records (32 and 48 bytes), one
   Diffie-Hellman operation per side where XX has three, one round trip
   less.
8. Parsing before authentication. Smaller than A.
9. Auditability. Maps directly onto the SIGMA literature and onto the
   structure of TLS 1.3. No degenerate use of a Noise feature to explain.
10. Key storage. None beyond the identity key.
11. Rotation. As A.

## Option C: persistent X25519 static key certified by the identity

Each identity has a long-lived X25519 key. The Ed25519 identity signs it.
Two variants: C1, XX with the certificate in the handshake payload, as
libp2p specifies; C2, XK or IK with the responder's static key published in
the contact card.

1. Authentication. Noise is used as designed: the static key is the
   long-term transport identity and the handshake authenticates it. The
   Ed25519 identity is reached through a certificate, a signature over a
   prefix and the static key.
2. Forward secrecy. Yes against later compromise of static keys. In IK the
   first payload has weaker protection.
3. Identity privacy. C1: the responder's certificate in message 2 goes to
   any prober, unless it is deferred. C2 with XK: the responder's static key
   is never transmitted, and the initiator's key and certificate are sent
   encrypted to an authenticated responder. This is the strongest identity
   hiding of the four options and it comes from Noise itself.
4. Probing. C2: a party without the responder's static key cannot complete
   the handshake. The static key is in the contact card and is public data
   in the same sense as the identity key.
5. Custom constructions. One: the certificate. Its format can be copied
   from the libp2p specification. It creates a standing credential,
   however: a stolen static private key plus one certificate impersonates
   the identity for as long as contacts accept that static key. That needs
   an expiry or revocation rule, which libp2p does not have and Monolith
   would have to design.
6. Dependencies. As A.
7. Complexity. Highest of the Noise options. A second long-term secret, its
   rotation, a larger contact card, certificate validation, and rules for
   contacts to learn a new static key.
8. Parsing before authentication. Fixed-size if the certificate has a fixed
   layout.
9. Auditability. Good. The formal analyses of XX, XK and IK apply directly,
   and the certificate step is conventional. Lightning and I2P use Noise
   with long-term static keys; libp2p uses exactly C1.
10. Key storage. One more persistent secret in the vault, to protect and to
    back up.
11. Rotation. The static key becomes something a contact must learn and
    update, like an endpoint. Card distribution and transport key lifetime
    become coupled.

## Option D: TLS 1.3, mutual authentication, pinned raw public keys

TLS 1.3 with RFC 7250 raw public keys on both sides. The Ed25519 identity
key is the TLS authentication key. Pinning is done in custom verifiers.

1. Authentication. CertificateVerify: each side signs the transcript with
   its identity key. This is part of TLS. Monolith writes no cryptographic
   composition, only the decision whether a presented key is the pinned
   one.
2. Forward secrecy. Yes with ephemeral key exchange. Resumption, session
   tickets and early data have to be disabled and kept disabled.
3. Identity privacy. As RFC 8446 states it: the server's identity is
   protected against passive attackers, the client's against passive and
   active ones. The server sends its key to any client that connects. This
   is the same shape as A without the prologue.
4. Probing. Any party that reaches the listener learns the server's
   identity key. Requiring something first would need an external
   pre-shared key; whether that is practical is not established.
5. Custom constructions. None cryptographic. Custom code: one verifier per
   side and the configuration that switches off what is not needed.
6. Dependencies. `rustls`: widely deployed, actively maintained, audited in
   2020, advisories handled promptly. Its providers (`ring`, `aws-lc-rs`)
   contain C and assembly with long deployment history and the review
   lineage of BoringSSL and AWS-LC. The dependency surface is larger than
   that of `snow`.
7. Complexity. Moderate, and of a different kind: configuration and
   verifier code instead of protocol design. Monolith's framing and padding
   are still needed inside the stream.
8. Parsing before authentication. The whole TLS 1.3 handshake: a state
   machine and extension parsing with variable-size messages. It is the
   largest surface in this comparison and also the most tested and fuzzed.
   It is written in memory-safe Rust above the provider.
9. Auditability. Best. TLS 1.3 has been analyzed extensively and reviewers
   know it. The implementation has an audit. What is specific to Monolith is
   small.
10. Key storage. None beyond the identity key. The library signs through an
    interface, so the key can stay in Monolith's own type.
11. Rotation. As A.

Other: the ClientHello is characteristic of the library and its version. It
is visible to the peer only, since the stream is inside Tor.

## Summary

| | A: XX + proof | B: NN + proof | C: certified static | D: TLS 1.3 |
| --- | --- | --- | --- | --- |
| Authentication comes from | Monolith's proof | Monolith's proof | Noise, via a certificate | TLS |
| Forward secrecy | yes | yes | yes | yes, resumption off |
| Responder identity shown to | holder of its public key | anyone connecting | C1 anyone, C2 holder of its static key | anyone connecting |
| Initiator identity shown to | verified responder | verified responder | authenticated responder | authenticated server |
| Custom constructions | 1 | 1 or 2 | 1, plus credential lifetime rules | 0 |
| Main dependency | `snow` | `snow` | `snow` | `rustls` and a provider |
| Audit of main dependency | none | none | none | 2020 |
| Bytes parsed before authentication | fixed, about 1.2 KiB | fixed, less | fixed | variable, bounded by the library |
| Extra long-term secrets | none | none | one | none |

## The prologue is not access control

Option A puts the responder's identity public key into the Noise prologue,
so that an initiator has to know that key to complete the handshake.

A public key is public data. Knowing it is not possession of a secret, and
this mechanism is not access control, not authorization and not
authentication of the initiator. It must not be described as strong
protection against probing. What it does is narrower: completing the normal
Monolith handshake requires knowledge of the expected responder identity,
which keeps a party that has only the onion address from being handed an
identity proof. Call it opportunistic probing resistance.

Its practical value is also smaller than Phase 0 implied. The onion address
is distributed in the contact card, and the card contains the identity key.
The two normally travel together, so the set of parties that know the
address but not the key is small.

If Monolith ever requires a real secret before it does anything expensive or
identifying, the mechanism for that is a high-entropy secret such as the
invitation capability, not the identity key. The invitation capability is
an anti-spam and anti-probing token. It is checked after authentication
today, and it is not an identity. Whether to move a secret of that kind in
front of the handshake (for example as a pre-shared key) is an open
question listed below.

## Assessment so far

Nothing is selected. What can be said:

- Ranked only by the amount of novel security-critical composition, D comes
  first with none, A and B follow with one small construction, and C has
  one construction plus rules for a long-lived credential.
- A and B rest on the same proof. They differ in what the Noise layer
  appears to claim. B claims less and is easier to explain; A keeps the
  prologue precondition.
- The difference in probing behavior between A and D is real but, for the
  reason given above, worth less than Phase 0 assumed.
- D's costs are the size of the pre-authentication parser, the custom
  verifiers, and a larger dependency tree. Its benefits are the depth of
  analysis and the audit history. Neither side of that trade has been
  measured yet.

`CRYPTOGRAPHY.md` keeps describing option A, as the candidate that happens
to be written up. That is a documentation state, not a decision.

## What has to happen before Phase 2

1. Decide the privacy requirement. Is it acceptable that a responder shows
   its identity key to any party that reaches the listener? If not, specify
   the gate, and make it a secret.
2. Check the practicality of D with a throwaway prototype that is not
   merged: mutual raw public keys in `rustls`; a server-side verifier that
   accepts any client key and leaves classification to the application;
   no server name; resumption and tickets off; how unknown or malformed
   keys are handled; the largest handshake the library will buffer; whether
   an exporter is available for later use.
3. If a Noise option is kept, choose between A and B explicitly, decide on
   C, and choose the crypto back end by the criteria above.
4. Have the chosen construction reviewed by a cryptographer.
5. Record the decision here and bring `CRYPTOGRAPHY.md` and `PROTOCOL.md`
   in line with it.

## What Phase 1 does in the meantime

- The outer frame is a length-prefixed opaque payload. The code takes the
  per-frame overhead of the session layer as a parameter (16 bytes for an
  AEAD tag under Noise, none if the stream is already protected).
- The handshake record sizes and the AuthProof layout in `PROTOCOL.md` are
  marked provisional. Phase 1 implements neither.
- No session library is added as a dependency.

## Risks that hold for any Noise option

- `snow` has no formal audit and does not zeroize key material. Issues
  about zeroization have been open since 2017.
- `snow` releases are slow, and it depends on the previous generation of
  the RustCrypto and dalek crates.
- The handshake hash is available only before the transition to transport
  mode.

## Open questions

These are the questions that remain for the session layer. They are also
listed in `CRYPTOGRAPHY.md` section 11.

- Q1. Which of A, B, C, D.
- Q2. Is disclosure of the responder's identity key to any connecting party
  acceptable, and if not, which secret gates it and where.
- Q3. Is the prologue precondition of A worth keeping, given what it is.
- Q4. For A and B: is the proof input sufficient and unambiguous.
- Q5. For A: is there any reason for XX over NN other than the prologue
  precondition.
- Q6. For C: what bounds the lifetime of a certificate.
- Q7. For D: the practicality points in step 2 above.
- Q8. Should the authentication also bind the onion service key that was
  dialed.
- Q9. Session limits and "reconnect, do not rekey".
- Q10. Crypto back end, and the zeroization gap in `snow`.

## Sources

Accessed 2026-10-01.

- Noise specification, revision 34: https://noiseprotocol.org/noise.html
- libp2p Noise: https://github.com/libp2p/specs/blob/master/noise/README.md
- libp2p TLS: https://github.com/libp2p/specs/blob/master/tls/tls.md
- RFC 8446: https://www.rfc-editor.org/rfc/rfc8446
- RFC 7250: https://www.rfc-editor.org/rfc/rfc7250
- snow: https://github.com/mcginty/snow , https://docs.rs/snow/0.10.0
- https://rustsec.org/advisories/RUSTSEC-2024-0011.html
- clatter: https://github.com/jmlepisto/clatter
- rustls features: https://docs.rs/rustls/0.23.45/rustls/manual/_04_features/
- rustls audit: https://cure53.de/pentest-report_rustls.pdf
- https://rustsec.org/packages/rustls.html
- Lightning transport (Noise XK): https://github.com/lightning/bolts/blob/master/08-transport.md
- I2P NTCP2 (Noise XK): https://i2p.net/en/docs/specs/ntcp2
- CVE-2022-24759, signature validation failure in a libp2p Noise
  implementation: https://osv.dev/vulnerability/CVE-2022-24759
- Noise Explorer: https://eprint.iacr.org/2018/766
- A Spectral Analysis of Noise:
  https://www.usenix.org/conference/usenixsecurity20/presentation/girol
