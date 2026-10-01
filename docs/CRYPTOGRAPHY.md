# Cryptography

Status: provisional. No cryptographic code exists. The session layer
(sections 4 to 7) is not decided: ADR 0002 compares four constructions and
selects none. Sections 4 to 7 describe one of them, option A, because it is
the one that has been written up in detail. Identity keys (section 3) and
the storage design (section 9) do not depend on that choice.

Monolith defines no primitive, no key exchange and no key schedule of its
own. It uses established protocols and Ed25519 as specified, and standard
hashes, KDFs and AEADs through maintained implementations. In option A the
one piece of composition that Monolith itself is responsible for is the
identity proof in section 5. The persistent identity is not the Noise
static identity there; long-term authentication comes from that proof,
layered on top of Noise. It is Monolith's construction and has to be
reviewed as one.

Tor provides anonymity and transport. Everything below is in addition to
Tor's own onion service encryption and does not replace any of it.

## 1. Primitives

| Purpose | Primitive | Implementation (candidate) |
| --- | --- | --- |
| Identity signatures | Ed25519, RFC 8032, PureEdDSA | `ed25519-dalek` |
| Session handshake and transport (provisional, option A) | `Noise_XX_25519_ChaChaPoly_SHA256`, Noise revision 34 | `snow`; crypto back end not chosen |
| Fingerprint, file digest | SHA-256 | `sha2` |
| Onion address checksum | SHA3-256 | `sha3` |
| Vault key derivation | Argon2id, RFC 9106 | `argon2` |
| Vault encryption | XChaCha20-Poly1305 | `chacha20poly1305` |
| Subkeys from the vault key | HKDF-SHA256, RFC 5869 | `hkdf` |
| Randomness | operating system CSPRNG | `getrandom` |
| Secret comparison | constant time | `subtle` |
| Secret erasure | | `zeroize` |

Hash choice for Noise: the specification states no preference between
SHA-256 and BLAKE2s. SHA-256 is chosen because it is the more widely
analyzed function, because `Noise_XX_25519_ChaChaPoly_SHA256` is the exact
suite libp2p deploys at scale, and because SHA-2 is in the dependency tree
anyway through Ed25519. BLAKE2s would add a hash for no benefit.

Dependency selection is recorded in `DEPENDENCIES.md` for crates that are
in use, and in ADR 0002 and ADR 0005 for candidates. A library is judged on
audit history, deployment history, cryptographic review, exposure of
memory-unsafe code to hostile input, parser complexity, dependency surface,
maintenance and published advisories. Being written in Rust, or containing
C or assembly, decides nothing by itself.

## 2. Randomness

Everything Monolith generates itself (identity keys, per-connection static
keys, identifiers, nonces, capabilities, isolation tokens) comes from the
operating system CSPRNG through one function in `monolith-identity`. If the
source reports an error, the operation fails; there is no fallback
generator. Non-cryptographic generators are banned by `deny.toml`, and will
be banned by lint once code exists that could call one (S17).

The Noise implementation generates the ephemeral handshake keys through its
own resolver, which also reads the operating system CSPRNG. Whether to route
that through Monolith's function with a custom resolver is decided in
Phase 2.

Jitter for reconnect timing and ping intervals uses the same source. It does
not need to be unpredictable, but one source is simpler to audit than two.

## 3. Identity keys

- A Monolith identity is an Ed25519 key pair. The public key is the
  identity. It is generated from 32 bytes of CSPRNG output.
- The private key is stored only in the encrypted vault (persistent mode) or
  in memory (ephemeral mode). It is never exported in plaintext, never
  logged, and held in a type that zeroizes on drop and has no `Debug`
  output.
- The identity key signs exactly two kinds of message, each with a fixed
  ASCII prefix:

  | Prefix | Total length of signed input | Defined in |
  | --- | --- | --- |
  | `MONOLITH-AUTH-V1` | 155 bytes | PROTOCOL.md 6.1 |
  | `MONOLITH-CONTACT-CARD-V1` | 99 or 115 bytes with one endpoint | PROTOCOL.md 11.1.1 |

  The prefixes differ and no input of one kind has the length of the other,
  so a signature made for one purpose cannot be presented for the other.
  Neither input can be mistaken for a bare 32-byte handshake hash.
- The identity key is used for signatures only. It is never converted to a
  Curve25519 key and never used in a Diffie-Hellman operation.
- Verification uses strict rules everywhere (`verify_strict`): the signature
  scalar must be canonical, and public keys or R values of small order are
  rejected. Plain RFC 8032 verification accepts some of these and is not
  used.
- A public key is valid only if it decompresses, is torsion-free and is
  not of small order (PROTOCOL.md section 10.1). Both of the last two
  tests are needed: the identity element is torsion-free. The rule applies
  to identity keys and to onion service keys, and is stricter than what
  signature verification requires, on purpose.
- The identity key is distinct from the Onion Service key and from every
  Noise key. A contact card that names its own identity key as an endpoint
  is not signed and not accepted (PROTOCOL.md section 11.2). Compromise of the Onion Service key lets an attacker receive
  connections at that address; it does not let the attacker pass the
  identity proof.

## 4. Session handshake (provisional, option A)

Pattern: Noise XX.

    -> e
    <- e, ee, s, es
    -> s, se

Parameters:

- DH: X25519. Cipher: ChaCha20-Poly1305. Hash: SHA-256.
- Prologue (40 bytes): preamble || responder identity public key. The
  preamble carries the protocol major version and is the same 8 bytes in
  both directions, so any tampering with it makes the handshake fail. The
  prologue is hashed into `h`; it is not secret and is not mixed into keys.
- Static keys `s`: a fresh X25519 pair generated per connection on each
  side. Never stored, never reused, bound to no identity.
- Ephemeral keys `e`: generated per connection by the Noise implementation.
- Handshake payloads: empty.

With empty payloads the three messages are exactly 32, 96 and 64 bytes.

What the handshake alone gives, from the Noise specification's analysis of
XX: after message 3, transport messages are encrypted with forward secrecy
to a party that proved possession of the static key it sent. Since the
static keys here are single-use and mean nothing by themselves, the
handshake establishes a confidential, forward-secret channel between two
parties who have not yet shown who they are. Identity comes from section 5.

What XX adds over a two-message pattern with no static keys (option B):
the third message is encrypted with `h` as associated data, so the
responder learns that the initiator computed the same `h`, including the
same prologue. Completing the normal handshake therefore requires knowledge
of the expected responder identity key, and a party that has only the
onion address is not handed an identity proof.

This is opportunistic probing resistance and nothing more. The identity key
is public data. Knowing it is not possession of a secret, so this is not
access control, not authorization and not authentication of the initiator.
The key is in every contact card next to the onion address, so the two
usually travel together. If a real secret is ever required before the
responder does anything identifying or expensive, it has to be a secret,
such as the invitation capability; ADR 0002 lists that as an open question.

A pattern in which the responder's static key is known in advance (XK, IK)
is option C in ADR 0002. It needs a long-lived Noise static key published
in the contact card, a new persistent secret with its own rotation rules.

## 5. Identity proof (provisional, options A and B)

### 5.1 Construction

After the handshake each side sends one AuthProof inside the encrypted
channel (PROTOCOL.md section 6). It contains the sender's identity public
key, a feature bit field and an Ed25519 signature over:

    "MONOLITH-AUTH-V1"            16 bytes
    role of the signer             1 byte   0x01 initiator, 0x02 responder
    protocol major version         2 bytes
    handshake hash h              32 bytes
    signer's Noise static key     32 bytes
    peer's Noise static key       32 bytes
    signer's identity public key  32 bytes
    features                       8 bytes

The responder sends first. The initiator verifies the responder's proof and
checks the identity against the one it dialed before it sends its own.

### 5.2 Where this comes from

The Noise specification, section 11.2, describes this use: after a
handshake, parties "can then sign the handshake hash ... to get an
authentication token which has a 'channel binding' property: the token
can't be used by the receiving party with a different session." Section 14
says a higher-level protocol should bind to `h` and not to the chaining
key.

Structurally it is the SIGMA pattern that TLS 1.3 uses in CertificateVerify:
an unauthenticated ephemeral key exchange, then a signature by each party's
long-term key over the transcript hash with a role-specific context, sent
under keys derived from the exchange.

The alternative that libp2p deploys signs the Noise static key instead of
the transcript and carries the signature in the handshake payloads (option
C in ADR 0002). Its drawback is that the signature is a long-lived
credential: whoever obtains a static private key and one captured payload
can impersonate the identity for as long as the static key is accepted,
and there is no expiry. A signature over `h` is valid for one session and
delegates nothing. Its advantage is that it uses Noise static keys for what
they were designed for.

Neither construction has a published formal analysis as a composition with
Noise XX. This one is small and follows the specification's own description,
but it is Monolith's composition and has to be reviewed as such.

### 5.3 What each field is for

- Prefix: separates this signature from every other use of the identity key.
- Role: both sides sign over the same `h`. Without the role a proof could be
  reflected back to its sender.
- Version: redundant with the prologue, which already covers the preamble.
  Kept so that the signed input is self-describing.
- `h`: covers the prologue, both ephemeral keys and both encrypted static
  keys. It is unique to the session. This is what stops a proof from being
  moved to another session.
- Static keys: redundant with `h`, which covers their ciphertexts. Included
  explicitly so that the binding to the Noise session does not rest on
  reasoning about encrypted values.
- Identity key: binds the signature to the claimed key explicitly, which
  rules out an attacker presenting someone else's signature under a
  different key that also verifies it.
- Features: authenticates the feature negotiation.

The input has a fixed length and fixed field positions. There is exactly one
way to encode it.

### 5.4 Informal security argument

- Relay by a man in the middle. An attacker between A and B runs one Noise
  session with each. The two sessions have different ephemeral keys and
  therefore different `h`. A proof made for one does not verify in the
  other.
- Replay. A recorded proof is bound to an old `h`. A new session has new
  ephemeral keys on the verifier's side, so `h` differs.
- Unknown key-share. For B to attribute A's session to an attacker M, M
  would need a signature by M's key over that session's `h` and static
  keys. M can produce one only for sessions it is an endpoint of.
- Reflection. Prevented by the role byte and by the rule that a peer's
  identity must differ from one's own.
- Key compromise impersonation. Learning A's identity private key lets the
  attacker impersonate A. It does not let the attacker impersonate B to A,
  which needs B's signature.
- Probing. Section 4: the responder sends its proof only after message 3
  verified, that is, only to an initiator that used its identity key. That
  key is public data, so this limits who is handed a proof by accident; it
  does not keep anyone out. The authentication tags in message 2 depend on
  `h`, so a prober that holds a list of candidate identity keys can check,
  offline, which of them is served at an address.
- Disclosure order. The initiator reveals its identity only after it has
  verified the responder's. An attacker who took over the Onion Service key
  but not the identity key therefore does not learn who tries to connect.

### 5.5 What is not claimed

- Deniability. A proof is a signature over a transcript. It does not prove
  what was said, but Monolith makes no deniability claim of any kind.
- Post-quantum security. X25519 and Ed25519 are not quantum-resistant.
  Recorded sessions could be decrypted by a future quantum computer. Tor's
  own onion service cryptography has the same limitation today.
- Post-compromise security. If an identity private key is stolen, the thief
  can impersonate that identity until contacts are told out of band. There
  is no automatic healing and no revocation mechanism in version 1.
- Protection against a compromised endpoint. Keys in the memory of a
  compromised machine are compromised.

## 6. Transport

- Each direction has its own cipher state from the Noise `Split()`.
- One frame is one Noise transport message: ChaCha20-Poly1305, 64-bit
  counter nonce starting at zero, empty associated data.
- Frames are processed strictly in order on a reliable stream. A frame that
  fails authentication ends the session at once, so an attacker gets one
  forgery attempt per session.
- Length is the only thing visible outside the encryption, and it is
  quantized by padding (PROTOCOL.md section 5).

## 7. Session lifetime and rekeying

A session ends, and a new handshake replaces it, when the first of these is
reached:

| Limit | Value |
| --- | --- |
| Age | 24 hours |
| Frames sent in one direction | 2^32 |
| Ciphertext bytes sent in one direction | 2^40 |

Noise allows 2^64 - 1 messages per cipher state. The frame limit stays a
factor of 2^32 below that.

The age limit is deferred while a file transfer is active, so that the
clock does not cut a transfer off, but never beyond 48 hours in total. No
new transfer starts on a session that is past 24 hours.

Both sides count. A receiver that sees its peer exceed the frame or byte
limit treats that as a violation.

There is no in-band rekey. The Noise `Rekey()` function is not used. A
reconnect is a complete new handshake with new ephemeral and static keys,
which is simpler to reason about than a rekeyed session and gives new
forward secrecy. The side that reaches a limit sends Close and dials again;
unacknowledged messages are resent on the new session.

The limits are constants, and tests run with reduced values to exercise the
path.

## 8. Forward secrecy and key lifetime

| Key | Lifetime | Stored |
| --- | --- | --- |
| Identity private key | until the user discards the identity | vault, or memory in ephemeral mode |
| Onion Service private key | until the endpoint is retired | vault, or memory in ephemeral mode |
| Noise static key pair | one connection | never |
| Noise ephemeral key pair | one handshake | never |
| Session cipher keys | one session, at most 24 hours | never |
| Vault key | while the vault is unlocked | never |

Compromise of the identity key or the Onion Service key does not reveal
past sessions: session keys derive from ephemeral Diffie-Hellman only.

Erasure: Monolith's own secret types zeroize on drop. `snow` does not
zeroize its internal state (see ADR 0002). Until that changes, session keys
and handshake secrets may remain in freed memory until it is reused. This
is listed as a known weakness, not hidden.

Secrets are not locked into RAM. Doing so needs `mlock`, which needs
`unsafe` or a dependency that wraps it. On a system with swap, secrets can
reach the swap device. Tails has no swap. The recommendation for other
systems is encrypted swap or none; `monolith doctor` reports the swap
state.

## 9. Storage encryption

Specified in `STORAGE.md`. In short: Argon2id derives a key from the
passphrase, that key unwraps a random vault key, and the vault is encrypted
with XChaCha20-Poly1305 with the header as associated data.

## 10. Implementation rules

If a Noise option is chosen:

- Noise is driven only through the library's API. No Monolith code
  implements a Noise step.
- With `snow`, the handshake hash is read from the handshake state before
  it is converted to transport state; it is not exposed afterwards. Version
  0.9.5 or later is required (RUSTSEC-2024-0011).
- The crypto back end is chosen by the criteria in section 1. No back end
  is excluded or preferred for the language it is written in.
- The exact pattern string is a constant, and a test asserts it.

In any case:

- Ed25519 verification is always `verify_strict`. A lint will ban the
  non-strict `verify` in Monolith crates once the dependency is added.
- Test vectors with fixed keys are committed for every signed structure and
  for a complete handshake (PROTOCOL.md section 16).

## 11. Open questions

The same list as in ADR 0002. All of them are open.

Q1. Which construction: Noise XX with per-connection static keys and a
    transcript signature (A), Noise NN with transcript signatures (B), a
    persistent X25519 static key certified by the identity (C), or TLS 1.3
    with pinned raw public keys (D).

Q2. Is it acceptable that a responder shows its identity key to any party
    that reaches the listener? If not, which secret gates that, and where
    in the handshake.

Q3. Is the prologue precondition of option A worth keeping, given that it
    rests on public data.

Q4. For A and B: is the proof input in section 5.1 sufficient and free of
    ambiguity, and is anything in it harmful to include.

Q5. For A: is there any reason for XX over NN other than the prologue
    precondition.

Q6. For C: what bounds the lifetime of a certificate over a static key.

Q7. For D: can `rustls` do mutual raw public keys with a server that
    accepts any client key, no server name, and resumption off; what does
    it buffer before authentication; is an exporter available.

Q8. Should the authentication also bind the onion service key that was
    dialed (PROTOCOL.md open question P2).

Q9. Are the session limits in section 7 reasonable, and is "reconnect, do
    not rekey" acceptable.

Q10. Which crypto back end, and whether the absence of zeroization in
     `snow` is acceptable for an audit candidate.

## 12. Sources

Accessed 2026-10-01.

- Noise Protocol Framework, revision 34: https://noiseprotocol.org/noise.html
- libp2p Noise specification:
  https://github.com/libp2p/specs/blob/master/noise/README.md
- TLS 1.3, RFC 8446, sections 4.4.3 and E.1:
  https://www.rfc-editor.org/rfc/rfc8446
- Noise Explorer: https://eprint.iacr.org/2018/766
- A Spectral Analysis of Noise:
  https://www.usenix.org/conference/usenixsecurity20/presentation/girol
- Flexible Authenticated and Confidential Channel Establishment:
  https://eprint.iacr.org/2019/436
- Ed25519 strict verification:
  https://docs.rs/ed25519-dalek/latest/ed25519_dalek/struct.VerifyingKey.html
- Argon2, RFC 9106: https://www.rfc-editor.org/rfc/rfc9106
- XChaCha20-Poly1305:
  https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
- snow: https://github.com/mcginty/snow and
  https://rustsec.org/advisories/RUSTSEC-2024-0011.html
