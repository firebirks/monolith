# Cryptography

Status: draft for review by an independent cryptographer. No cryptographic
code exists yet. Phase 2 implements this document.

Monolith defines no primitive, no key exchange and no key schedule of its
own. It uses the Noise Protocol Framework as specified, Ed25519 as
specified, and standard hashes, KDFs and AEADs through maintained Rust
implementations. The one piece of composition that Monolith itself is
responsible for is the identity proof in section 5. It follows a
construction the Noise specification describes, and it is the part of this
document that most needs outside review.

Tor provides anonymity and transport. Everything below is in addition to
Tor's own onion service encryption and does not replace any of it.

## 1. Primitives

| Purpose | Primitive | Implementation (candidate) |
| --- | --- | --- |
| Identity signatures | Ed25519, RFC 8032, PureEdDSA | `ed25519-dalek` |
| Session handshake and transport | `Noise_XX_25519_ChaChaPoly_SHA256`, Noise revision 34 | `snow` with its default pure-Rust resolver |
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

Dependency selection, versions, audit status and known advisories are in
ADR 0002 and ADR 0005.

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
  | `MONOLITH-CONTACT-CARD-V1` | 98 or 114 bytes | PROTOCOL.md 11.1 |

  The prefixes differ and no input of one kind has the length of the other,
  so a signature made for one purpose cannot be presented for the other.
  Neither input can be mistaken for a bare 32-byte handshake hash.
- The identity key is used for signatures only. It is never converted to a
  Curve25519 key and never used in a Diffie-Hellman operation.
- Verification uses strict rules everywhere (`verify_strict`): the signature
  scalar must be canonical, and public keys or R values of small order are
  rejected. Plain RFC 8032 verification accepts some of these and is not
  used. A public key must also be canonically encoded; PROTOCOL.md section
  10.1 gives the exact tests.
- The identity key is distinct from the Onion Service key and from every
  Noise key. Compromise of the Onion Service key lets an attacker receive
  connections at that address; it does not let the attacker pass the
  identity proof.

## 4. Session handshake

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

Why XX and not a two-message pattern with no static keys: the third message
is encrypted with `h` as associated data, so the responder learns that the
initiator computed the same `h`, including the same prologue. The initiator
therefore must already know the responder's identity key. A party that
knows only the onion address cannot complete the handshake, and the
responder has not yet sent anything that reveals its identity to someone
who does not already hold the key (section 5.4, probing).

Why not a pattern in which the responder's static key is known in advance
(XK, IK): that would require a long-lived Noise static key published in the
contact card, a new persistent secret with its own rotation rules. The
prologue binding gives the property that matters here, that probes without
the card learn nothing, without one.

## 5. Identity proof

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
the transcript and carries the signature in the handshake payloads. It was
not chosen because that signature is a long-lived credential: whoever
obtains a static private key and one captured payload can impersonate the
identity for as long as the static key is accepted, and there is no expiry.
A signature over `h` is valid for one session and delegates nothing.

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
  verified, that is, only to an initiator that knew its identity key.
  One thing remains possible: the authentication tags in message 2 depend
  on `h`, so a prober that holds a list of candidate identity keys can
  check, offline, which of them is served at an address. It learns nothing
  about a key that is not on its list.
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

- Noise is driven only through the `snow` API. No Monolith code implements
  a Noise step.
- The handshake hash is read from the handshake state before it is
  converted to transport state; `snow` does not expose it afterwards.
- `snow` 0.9.5 or later is required (RUSTSEC-2024-0011).
- The `ring` resolver is not used. The default resolver is pure Rust.
- The exact pattern string is a constant, and a test asserts it.
- Ed25519 verification is always `verify_strict`. A lint will ban the
  non-strict `verify` in Monolith crates once the dependency is added.
- Test vectors with fixed keys are committed for every signed structure and
  for a complete handshake (PROTOCOL.md section 16).

## 11. Questions for the reviewer

Q1. Is the AuthProof input sufficient and free of ambiguity? Is anything in
    it harmful to include?

Q2. Is binding the responder's identity key through the prologue sound for
    the stated purpose (no identity disclosure to a party that does not
    already know the key)? Is there a reason to prefer a PSK modifier or an
    XK-style pattern?

Q3. Fresh static keys per connection make the `s` tokens of XX carry no
    long-term meaning. Is there any downside compared with an NN handshake
    followed by the same proofs, apart from the prologue confirmation that
    motivates XX here?

Q4. Should the proofs also bind the onion service key that was dialed
    (PROTOCOL.md open question P2)?

Q5. Are the session limits in section 7 reasonable, and is "reconnect, do
    not rekey" acceptable?

Q6. Is the absence of zeroization in `snow` acceptable for an audit
    candidate, or must it be fixed upstream or worked around first?

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
