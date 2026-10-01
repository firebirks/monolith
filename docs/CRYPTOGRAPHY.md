# Cryptography

Status: the session layer is not decided and not implemented. ADR 0002
has two finalists. Sections 4 to 8 and 10 of this document, and the parts
of sections 1 to 3 that speak of TLS, are a draft for one of them,
finalist D (TLS 1.3 with raw public keys on both sides). If the other
finalist, Noise XK with a transport key in the contact card, is chosen,
they are rewritten. Identity keys (section 3) are implemented. The storage
design (section 9) is specified separately and belongs to a later phase.

Monolith defines no primitive, no key exchange, no key schedule and no
authentication protocol of its own. It uses TLS 1.3 and Ed25519 as
specified, and standard hashes, KDFs and AEADs through maintained
implementations. What Monolith itself decides in the session layer is
small: which key it accepts from a peer, and which parts of TLS are
switched off. Section 5 lists those decisions. They are Monolith's
responsibility and have not been reviewed by anyone outside the project.

Tor provides anonymity and transport. Everything below is in addition to
Tor's own onion service encryption and does not replace any of it.

## 1. Primitives

| Purpose | Primitive | Implementation |
| --- | --- | --- |
| Identity signatures | Ed25519, RFC 8032, PureEdDSA | `ed25519-dalek` |
| Session protocol | TLS 1.3, RFC 8446, raw public keys, RFC 7250 | `rustls` |
| Session key exchange | X25519 | `ring`, through `rustls` |
| Session key schedule | HKDF-SHA256, as TLS 1.3 defines it | `ring`, through `rustls` |
| Session record protection | ChaCha20-Poly1305 | `ring`, through `rustls` |
| Fingerprint, file digest | SHA-256 | `sha2` |
| Onion address checksum | SHA3-256 | `sha3` (not in use yet) |
| Vault key derivation | Argon2id, RFC 9106 | `argon2` (not in use yet) |
| Vault encryption | XChaCha20-Poly1305 | `chacha20poly1305` (not in use yet) |
| Subkeys from the vault key | HKDF-SHA256, RFC 5869 | `hkdf` (not in use yet) |
| Randomness | operating system CSPRNG | `ring` for the session; `getrandom` for Monolith's own values |
| Secret comparison | constant time, best effort | `subtle` |
| Secret erasure | best effort | `zeroize` |

One suite: `TLS_CHACHA20_POLY1305_SHA256`, with x25519 as the only group
and ed25519 as the only signature scheme. ChaCha20-Poly1305 is chosen over
AES-GCM because it runs in constant time in software on every processor
Monolith targets, has no record limit below the sequence space
(RFC 8446 section 5.5), and is the cipher family of the storage design.
There is no second suite and so no negotiation to protect.

Dependency selection is recorded in `DEPENDENCIES.md` for crates that are
in use, and in ADR 0002 and ADR 0005 for the ones that are selected and
not yet added. A library is judged on audit history, deployment history,
cryptographic review, exposure of memory-unsafe code to hostile input,
parser complexity, dependency surface, maintenance and published
advisories. Being written in Rust, or containing C or assembly, decides
nothing by itself.

## 2. Randomness

Everything Monolith generates itself (identity keys, identifiers,
capabilities, isolation tokens) comes from the operating system CSPRNG
through one function. If the source reports an error, the operation fails;
there is no fallback generator. Non-cryptographic generators are banned by
`deny.toml`, and will be banned by lint once code exists that could call
one (S17).

The TLS handshake needs random values and an ephemeral key. Both are
generated inside the provider, which reads the operating system CSPRNG.
Monolith passes no randomness into the session layer in production.

Tests that need a reproducible handshake use a provider built for tests.
It exists only in test code and cannot be selected by a feature of a
production crate.

Jitter for reconnect timing and ping intervals uses the operating system
source as well. It does not need to be unpredictable, but one source is
simpler to audit than two.

## 3. Identity keys

- A Monolith identity is an Ed25519 key pair. The public key is the
  identity. It is generated from 32 bytes of CSPRNG output.
- The private key is stored only in the encrypted vault (persistent mode) or
  in memory (ephemeral mode). It is never exported in plaintext, never
  logged, and held in a type that zeroizes on drop and has no `Debug`
  output.
- The identity key signs exactly two kinds of input:

  | Use | Signed input | Length | Defined in |
  | --- | --- | --- | --- |
  | Contact card | `MONOLITH-CONTACT-CARD-V1`, then the card fields | 99 or 115 bytes with one endpoint | PROTOCOL.md 11.1.1 |
  | TLS 1.3 CertificateVerify | 64 bytes 0x20, a context string, 0x00, a transcript hash | 130 bytes | RFC 8446 4.4.3, PROTOCOL.md 4.3 |

  The first begins with the byte 0x4D and the second with 0x20, and their
  lengths differ, so a signature made for one purpose cannot be presented
  for the other. TLS 1.3 chose its 64-byte prefix for exactly this
  separation from other uses of a key.
- The identity key is used for signatures only. It is never converted to a
  Curve25519 key and never used in a Diffie-Hellman operation.
- Verification uses strict rules everywhere (`verify_strict`): the signature
  scalar must be canonical, and public keys or R values of small order are
  rejected. Plain RFC 8032 verification accepts some of these and is not
  used. This includes the TLS signatures: Monolith verifies them itself and
  does not leave that to the TLS provider.
- A public key is valid only if it decompresses, is torsion-free and is
  not of small order (PROTOCOL.md section 10.1). Both of the last two
  tests are needed: the identity element is torsion-free. The rule applies
  to identity keys, to onion service keys and to the key a peer presents
  in a TLS handshake, and is stricter than what signature verification
  requires, on purpose.
- Key separation: the identity key is distinct from the Onion Service key
  and from every session key. Each is generated independently from its own
  CSPRNG output; none is derived from another or from a shared seed. The
  Onion Service master key is handed to Tor, which uses it under its own
  rules. Using one key in both domains would let a signature or a
  compromise in one carry over to the other.

  The protocol enforces what it can see: a contact card whose endpoint is
  byte for byte the identity key is invalid, and is never signed
  (PROTOCOL.md section 11.2, invariant S33).
- Using the identity key as the TLS authentication key is not a second
  domain in that sense. The key does what it exists for, proving the
  identity, and it signs only inputs that TLS 1.3 separates from every
  other use by construction. No second long-term key exists.
- Compromise of the Onion Service key lets an attacker receive connections
  at that address; it does not let the attacker authenticate as the
  identity.

## 4. Session handshake

The handshake is TLS 1.3 with the profile of PROTOCOL.md section 4: one
version, one suite, one group, raw public keys in both directions, client
authentication required, and no resumption, tickets or early data.

    Initiator (TLS client)                      Responder (TLS server)

      ClientHello                       ---->
                                                ServerHello
                                                {EncryptedExtensions}
                                                {CertificateRequest}
                                                {Certificate}
                                                {CertificateVerify}
                                        <----   {Finished}
      {Certificate}
      {CertificateVerify}
      {Finished}                        ---->
      [frames]                          <--->   [frames]

What the handshake gives is what RFC 8446 appendix E.1 states for the
full handshake with certificate authentication on both sides:

- both sides hold the same session keys, and only they do;
- each side has authenticated the other's key;
- session keys are unique to the session;
- an attacker cannot make the peers negotiate other parameters than they
  would without the attacker;
- forward secrecy: later compromise of an identity key does not reveal the
  keys of past sessions;
- resistance to key compromise impersonation: an attacker who holds A's
  identity key cannot pose as someone else towards A;
- protection of identities: the responder's key against passive attackers,
  the initiator's against passive and active ones.

These are properties of TLS 1.3. Monolith does not derive them. The
analyses behind them model keys that a party can authenticate; with raw
public keys the party is the key, and the binding to a Monolith identity
is the comparison in section 5.

## 5. Authentication of Monolith identities

### 5.1 What Monolith decides

The TLS end-entity key of each side is its Monolith identity key. There is
no certificate, no name, and no proof of Monolith's own design. Monolith
decides:

- Initiator: the responder's key must be the identity that was dialed.
- Responder: the initiator's key must be a valid key and not the
  responder's own. Nothing else is looked at, in particular no record of
  contacts or blocked identities.
- Both: the key is carried as the exact 44-byte Ed25519
  SubjectPublicKeyInfo and passes the validity rule of section 3; the
  signature in CertificateVerify verifies under strict rules; after the
  handshake the version, the suite and the ALPN identifier are the
  expected ones.

PROTOCOL.md section 4 gives these checks as rules, and ADR 0002 lists them
in order.

### 5.2 What is signed

Each side sends one CertificateVerify. Its signature is over 130 bytes
that RFC 8446 section 4.4.3 defines:

    64 bytes   0x20 repeated
    33 bytes   "TLS 1.3, server CertificateVerify" (responder) or
               "TLS 1.3, client CertificateVerify" (initiator)
     1 byte    0x00
    32 bytes   SHA-256 over the handshake messages so far, up to and
               including the signer's own Certificate message

The transcript binds the version, the suite, the group, both key shares,
both random values, the ALPN identifier and the certificate types. The
context string binds the role. The initiator's transcript also contains
the responder's key and signature.

### 5.3 Order and what each side reveals

The responder authenticates first. The initiator sends its key and its
signature only after it has verified the responder's key against the
identity it dialed, the responder's signature, and the responder's
Finished.

- The responder's identity key becomes known to whoever completes the
  first round trip. Any party that can reach the listener and speaks the
  profile obtains the key and a signature that shows the identity is live
  at this address.
- The initiator's identity key becomes known only to a responder that has
  proved the dialed identity. A party that answers at the address without
  holding that identity key learns that someone connected and nothing
  about who.
- A passive observer of the stream sees that TLS 1.3 is spoken and reads
  the ALPN identifier in the ClientHello. It sees no identity. Inside Tor
  only the endpoints see the stream.

Knowing the responder's public key was never a condition for anything.
ADR 0002, "Public keys are not secrets", explains what the earlier
proposal offered there and why it is not kept.

### 5.4 Informal security argument for the Monolith-specific part

The argument for the handshake itself is the literature on TLS 1.3. What
follows covers only what Monolith adds: the identity of a party is its
key, and the two verifiers.

- Man in the middle. An attacker between A and B has to run its own
  handshake with each. Towards A it must present B's key and sign a
  transcript with it, which it cannot. A stops and reveals nothing. Towards
  B it can only authenticate as itself.
- Replay. Every transcript contains fresh random values and key shares
  from both sides. A recorded signature fits no other transcript.
- Unknown key-share and misbinding. The classical attack makes one side
  attribute a session to the wrong party by substituting an identity next
  to a signature. Here there is no identity next to the key: the identity
  is the key that verified the signature over this transcript.
- Reflection. Client and server sign under different context strings, and
  each side refuses its own key.
- Wrong identity on an outbound session. The initiator's verifier accepts
  one key. Any other key ends the handshake before the initiator has sent
  anything of its own.
- Oracles. The responder's handshake does not depend on what it holds about
  the initiator, so its outcome and timing say nothing about the contact
  list. Every rejection by Monolith's checks looks the same to the peer.

### 5.5 What is not claimed

- Deniability. A session contains signatures over its transcript by both
  identity keys. They do not prove what was said, but Monolith makes no
  deniability claim of any kind.
- Post-quantum security. X25519 and Ed25519 are not quantum-resistant.
  Recorded sessions could be decrypted by a future quantum computer. Tor's
  own onion service cryptography has the same limitation today. A hybrid
  key exchange is a possible later protocol version, selected by a new
  ALPN identifier; it is not part of version 1.
- Post-compromise security. If an identity private key is stolen, the thief
  can impersonate that identity until contacts are told out of band. There
  is no automatic healing and no revocation mechanism in version 1.
- Protection against a compromised endpoint. Keys in the memory of a
  compromised machine are compromised.
- An audit. TLS 1.3 has been analyzed widely and `rustls` was audited in
  2020. Monolith's verifiers, its configuration of the library and its
  use of the identity key as the TLS key have been reviewed by nobody
  outside the project.

## 6. Transport

- After the handshake the TLS record layer protects everything. Monolith
  adds no encryption of its own.
- Monolith frames (PROTOCOL.md section 5) are written into the TLS stream
  as application data. A frame may span several TLS records, and record
  boundaries carry no meaning.
- TLS authenticates a sequence number with every record. A record that is
  replayed, dropped, reordered or taken from another session fails
  authentication, and one failure ends the session, so an attacker gets
  one forgery attempt per session.
- Lengths are what an observer of the stream could see, and they are
  quantized by padding (PROTOCOL.md section 5). A TLS record adds 22 bytes
  to its content.
- Monolith adds no sequence numbers. Repetition at the level of messages
  is handled by the messages: a chat message that is sent again after a
  reconnect keeps its MessageId and is delivered once, and a contact card
  that is sent again is judged by its epoch.

## 7. Session lifetime and rekeying

A session ends, and a new handshake replaces it, when the first of these is
reached:

| Limit | Value |
| --- | --- |
| Age | 24 hours |
| Frames sent in one direction | 2^32 |
| Plaintext bytes sent in one direction | 2^40 |

The cipher forces none of them. With ChaCha20-Poly1305, TLS 1.3 has no
confidentiality limit below its 2^64 record sequence numbers, and
integrity is not weakened by volume because a single failed record ends
the session. The limits bound how long one set of keys is used and make a
long-lived connection take new ephemeral keys regularly. In practice the
age limit is the one that is reached; the other two are far away at the
speed of a Tor stream and exist so that no counter is unbounded.

The age limit is deferred while a file transfer is active, so that the
clock does not cut a transfer off, but never beyond 48 hours in total. No
new transfer starts on a session that is past 24 hours.

Both sides count. A receiver that sees its peer exceed the frame or byte
limit treats that as a violation.

Monolith never rekeys a session. A reconnect is a complete new handshake,
which is simpler to reason about than a rekeyed session. The side that
reaches a limit sends Close and dials again; unacknowledged messages are
resent on the new session. A TLS KeyUpdate from the peer is legal and is
handled by the library; Monolith does not send one.

There is no session resumption and no early data. A responder issues no
tickets and keeps no session store; an initiator offers and stores none.

The limits are constants, and tests run with reduced values to exercise the
path.

## 8. Forward secrecy and key lifetime

| Secret | Generated | Lifetime | Stored | Erased |
| --- | --- | --- | --- | --- |
| Identity private key | by Monolith, from the OS CSPRNG | until the user discards the identity | vault, or memory in ephemeral mode | by its type on drop |
| Onion Service private key | by Tor or by Monolith | until the endpoint is retired | vault, or memory in ephemeral mode | by its type on drop |
| TLS ephemeral X25519 key | by the provider, from the OS CSPRNG | one handshake | never | not explicitly |
| TLS handshake and traffic secrets | by the TLS key schedule | one session, at most 24 hours | never | by `rustls` on drop |
| Record keys inside the provider | from the traffic secrets | one session | never | not explicitly |
| Invitation capability | by Monolith, from the OS CSPRNG | until revoked | vault | by its type on drop |
| Vault key | derived at unlock | while the vault is unlocked | never | by its type on drop |

None of the session secrets is cloned out of the library, and none is ever
written to storage. Types that hold a secret do not derive `Debug`.

Compromise of the identity key or the Onion Service key does not reveal
past sessions: session keys derive from an ephemeral exchange only.

Erasure: Monolith's own secret types zeroize on drop, and `rustls` does
the same for the secrets it holds. `ring` does not clear its internal key
state. Erasure removes the copies a type controls. It does not reach
copies the compiler made, freed memory that was reused, pages that were
swapped out, or crash dumps. This is listed as a limit, not hidden.

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

- TLS is driven only through the library. No Monolith code builds or
  parses a TLS message.
- One crate holds every call to the TLS library. Contact logic, storage
  and front ends do not depend on it.
- The version, the suite, the group, the signature scheme and the ALPN
  identifier are constants. After every handshake the negotiated values
  are compared with them; a test asserts each.
- TLS 1.2 is not compiled in. Resumption, tickets, early data, server name
  indication and certificate compression are off, and a test for each
  shows that it stays off.
- The identity private key stays in Monolith's key type. The library
  receives a signer, not the key.
- Ed25519 verification is always `verify_strict`, including for TLS
  signatures. A lint bans the non-strict `verify` in Monolith crates.
- The only way to obtain a session that can send or receive a frame is
  a completed handshake. The object it returns owns the session logic of
  the protocol core; callers are not handed a session they could mark
  authenticated themselves.
- Test vectors with fixed keys are committed for every structure Monolith
  defines (PROTOCOL.md section 16).

## 11. Open points

The questions Q1 to Q11 of earlier drafts are closed by ADR 0002. What
remains is listed there as risks R1 to R7: the size of the parser in front
of unauthenticated peers, the future of the provider, the announced change
of the library's interface, the age of its audit, deterministic handshake
vectors, disclosure of the responder's key to probers, and the memory an
unfinished handshake holds.

PROTOCOL.md open question P2, whether a session should also be bound to
the onion service key that was dialed, is unchanged and open.

## 12. Sources

Accessed 2026-10-01.

- TLS 1.3, RFC 8446, sections 4.4.3, 5.5 and appendix E.1:
  https://www.rfc-editor.org/rfc/rfc8446
- Raw public keys in TLS, RFC 7250: https://www.rfc-editor.org/rfc/rfc7250
- Ed25519 in SubjectPublicKeyInfo, RFC 8410:
  https://www.rfc-editor.org/rfc/rfc8410
- rustls: https://docs.rs/rustls/0.23.45
- SIGMA: https://www.iacr.org/archive/crypto2003/27290399/27290399.pdf
- Ed25519 strict verification:
  https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/struct.VerifyingKey.html
- Argon2, RFC 9106: https://www.rfc-editor.org/rfc/rfc9106
- XChaCha20-Poly1305:
  https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03

The sources for the comparison of session protocols are in ADR 0002.
