# Cryptography

Status: the session layer is decided and implemented. It is Noise XK with
a transport key that the identity certifies in the contact card (ADR
0002). Sections 4 to 8 describe it, and the crate `monolith-session`
implements it. The storage design (section 9) is specified separately and
belongs to a later phase.

Monolith defines no primitive, no key exchange and no key schedule of its
own. It uses the Noise Protocol Framework and Ed25519 as specified, and
standard hashes, KDFs and AEADs through maintained implementations.

What Monolith does own is the binding between Noise transport keys and
Monolith identities. It consists of five rules, listed in section 5.2.
They follow the method the Noise specification names for this purpose, a
certificate over the static key, but they are Monolith's composition.
Nobody outside the project has reviewed them, and they have to be judged
as what they are.

Tor provides anonymity and transport. Everything below is in addition to
Tor's own onion service encryption and does not replace any of it.

## 1. Primitives

| Purpose | Primitive | Implementation |
| --- | --- | --- |
| Identity signatures | Ed25519, RFC 8032, PureEdDSA | `ed25519-dalek` |
| Session handshake and transport | `Noise_XK_25519_ChaChaPoly_SHA256`, Noise revision 34 | `snow`, with the resolver of the session crate (ADR 0002, F-R1) |
| Session key exchange | X25519, RFC 7748 | `x25519-dalek` |
| Session cipher | ChaCha20-Poly1305, RFC 8439 | `chacha20poly1305` |
| Session hash and key derivation | SHA-256, HMAC-SHA256 in the Noise HKDF | `sha2`; HMAC and HKDF as `snow` builds them from it |
| Fingerprint, file digest | SHA-256 | `sha2` |
| Onion address checksum | SHA3-256 | `sha3` (not in use yet) |
| Vault key derivation | Argon2id, RFC 9106 | `argon2` (not in use yet) |
| Vault encryption | XChaCha20-Poly1305 | `chacha20poly1305` (not in use yet) |
| Subkeys from the vault key | HKDF-SHA256, RFC 5869 | `hkdf` (not in use yet) |
| Randomness | operating system CSPRNG | `getrandom` |
| Secret comparison | constant time, best effort | `subtle` |
| Secret erasure | best effort | `zeroize` |

One suite. There is no second choice for the Diffie-Hellman function, the
cipher or the hash, so there is nothing to negotiate.

Hash choice: the Noise specification states no preference between SHA-256
and BLAKE2s. SHA-256 is chosen because it is the more widely analyzed
function and because SHA-2 is in the dependency tree anyway through
Ed25519. BLAKE2s would add a hash for no benefit.

Dependency selection is recorded in `DEPENDENCIES.md` for crates that are
in use, and in ADR 0002 and ADR 0005 for the ones that are selected and
not yet added. A library is judged on audit history, deployment history,
cryptographic review, exposure of memory-unsafe code to hostile input,
parser complexity, dependency surface, maintenance and published
advisories. Being written in Rust, or containing C or assembly, decides
nothing by itself.

## 2. Randomness

Everything secret that is generated (identity keys, transport keys,
ephemeral handshake keys, capabilities, identifiers, isolation tokens)
comes from the operating system CSPRNG. If the source reports an error,
the operation fails; there is no fallback generator. Non-cryptographic
generators are banned by `deny.toml`, and will be banned by lint once code
exists that could call one (S17).

The source is read through `getrandom`, in the session crate and nowhere
else. The ephemeral keys of a handshake are generated there: the Noise
library asks the resolver for random bytes, and the resolver reads the
operating system source. In production there is no way to supply them
from outside.

Tests that need a reproducible handshake fix the ephemeral keys. The means
to do that is a second random source in the resolver that returns fixed
bytes. It is compiled only into the tests of the session crate and into
builds made with `--cfg fuzzing`, which is what `cargo fuzz` passes and
nothing else does. It is not a cargo feature, so no crate in a dependency
tree can switch it on. The test hook of the Noise library's builder for
fixed ephemeral keys is not used at all.

Jitter for reconnect timing and ping intervals uses the operating system
source as well. It does not need to be unpredictable, but one source is
simpler to audit than two.

## 3. Keys of an identity

A Monolith identity has three keys. They belong to three domains, are
generated independently, each from CSPRNG output of its own, and none is
derived from another or from a shared seed.

| Key | Type | Used for | Used by |
| --- | --- | --- | --- |
| Identity key | Ed25519 | signing contact cards, nothing else | Monolith, when a card is issued |
| Transport key | X25519 | authenticating sessions, as the Noise static key | the session layer, at every handshake |
| Onion key | Ed25519 | reachability of an Onion Service | Tor, under Tor's rules |

- The public identity key is the identity. A fingerprint (PROTOCOL.md
  section 10) is computed from it and from nothing else.
- The identity key signs exactly one kind of input: the signed bytes of a
  contact card, which begin with `MONOLITH-CONTACT-CARD-V1` and are 131 or
  147 bytes long in version 1 (PROTOCOL.md section 11.1.1). It is not
  needed to run a session. An implementation can keep it locked while it
  is connected.
- The identity key is used for signatures only. It is never converted to a
  Curve25519 key and never used in a Diffie-Hellman operation.
- The transport key is used for Diffie-Hellman in the Noise handshake and
  for nothing else. The Noise specification asks for exactly that: a
  static key pair should not be used outside Noise.
- Verification of Ed25519 signatures uses strict rules everywhere
  (`verify_strict`): the signature scalar must be canonical, and public
  keys or R values of small order are rejected.
- An Ed25519 public key is valid only if it decompresses, is torsion-free
  and is not of small order (PROTOCOL.md section 10.1). Both of the last
  two tests are needed: the identity element is torsion-free.
- An X25519 public key is valid only if it is canonically encoded and is
  not of small order (PROTOCOL.md section 10.2). This applies to transport
  keys and to the ephemeral keys received in a handshake.
- Key separation is enforced where a card can show a violation: an
  endpoint that is the identity key, or a transport key that is the
  identity key or an endpoint key in its other curve form, makes the card
  invalid, and no such card is signed (PROTOCOL.md section 11.2,
  invariant S33).
- Private keys are stored only in the encrypted vault (persistent mode) or
  in memory (ephemeral mode). They are never exported in plaintext, never
  logged, and held in types that zeroize on drop and have no `Debug`
  output.

What a stolen key allows:

| Stolen | The thief can | The thief cannot |
| --- | --- | --- |
| Identity private key | issue cards for the identity, with any transport key and endpoint: full impersonation | read past sessions |
| Transport private key | authenticate as the identity when it dials, and when it is dialed if it also controls an endpoint of the card | issue cards; read past sessions; act against a contact that has received a newer card |
| Onion private key | receive the connections made to that address | complete a handshake; learn who connects |

There is no revocation in version 1, for any of the three.

## 4. Session handshake

Pattern: Noise XK.

    <- s
    ...
    -> e, es
    <- e, ee
    -> s, se

PROTOCOL.md section 4 gives the parameters and the bytes. In short:

- Protocol name `Noise_XK_25519_ChaChaPoly_SHA256`.
- Prologue (51 bytes): the label `MONOLITH-SESSION-V1` and the responder's
  identity public key. The prologue is hashed into `h`; it is not secret
  and is not mixed into keys.
- Static keys: the transport keys. The initiator knows the responder's
  from the contact card it holds.
- Ephemeral keys: generated per handshake, never stored, never reused.
- Payloads: empty in messages 1 and 2; the initiator's contact card in
  message 3.

The three messages are exactly 48, 48 and 235 bytes.

What the handshake gives, in the terms of the Noise specification
(section 7.7) for XK:

| Message | Sender authentication | Confidentiality for the recipient |
| --- | --- | --- |
| 1 | none (0) | to a known recipient, forward secret only if the responder's transport key stays secret (2) |
| 2 | the sender holds the responder's transport key, resistant to key compromise impersonation (2) | to an ephemeral recipient (1) |
| 3 and every frame | the sender holds the static key it presented, resistant to key compromise impersonation (2) | to a known recipient, strong forward secrecy (5) |

Message 1 carries no payload, so its weaker confidentiality protects
nothing. The first data an initiator sends is the card in message 3.

Identity hiding (section 7.8 of the specification): the initiator's static
key is "encrypted with forward secrecy to an authenticated party" (8). The
responder's static key is "not transmitted, but a passive attacker can
check candidates for the responder's private key" (3).

Published analyses of XK: Noise Explorer (symbolic, with an active
attacker and malicious principals) and the fACCE analysis (computational).
They cover the authentication of the static keys. They do not cover the
binding of those keys to Monolith identities, which is section 5.

## 5. Identities

### 5.1 How an identity is bound to a session

Noise authenticates transport keys. The contact card says whose they are.

- Towards the initiator: the initiator holds the responder's card, signed
  by the responder's identity key, and uses the transport key in it.
  Message 2 proves that the answering party holds that transport key.
- Towards the responder: the initiator sends its own card in message 3.
  The responder verifies the card and checks that its transport key is the
  one Noise authenticated.

No signature is made during a session. The only signatures involved are
those of the two cards, made when the cards were issued.

### 5.2 The rules Monolith owns

F1. The card carries the transport key, and the card signature covers it.
    The card is the certificate; there is no second signed structure.

F2. The prologue contains the responder's identity key. A card shows that
    an identity vouches for a transport key. It does not show that the
    holder of the transport key agrees to be that identity. Without F2 an
    identity could sign a card naming another party's endpoint and
    transport key, and a peer that imported it would complete a handshake
    with that other party and attribute the session to the wrong
    identity. With F2 the initiator hashes the identity it believes it is
    dialing and the responder its own, and message 1 fails when they
    differ. This is the misbinding described for certified Diffie-Hellman
    keys in the SIGMA paper.

F3. The initiator's card travels inside the handshake, as the payload of
    message 3. The responder accepts the identity in it only if the card
    is valid, its transport key is byte for byte the static key Noise
    authenticated, and the identity is not the responder's own. Because
    the card is inside message 3, it is the holder of the transport key
    who presents it; that is the initiator's half of F2.

F4. A card that is older than the newest card the local side holds of
    that identity, or that has the same epoch and other contents, does not
    open a contact session. The peer is treated as an identity that is not
    a contact and is not told why. The newest card held is the pinned one
    or a later one that the user has not confirmed yet: what counts as
    stale does not wait for the user. A responder applies the rule to the
    card presented in message 3, an initiator to the card it dialed.

F5. A transport key or ephemeral key that is not canonically encoded or is
    of small order is invalid, and an X25519 result of all zeros ends the
    handshake. The Noise specification leaves this to the application.

### 5.3 What each party learns

| Party | Learns |
| --- | --- |
| Passive observer of the stream | Three messages of fixed size. Nothing readable. Inside Tor, only the two endpoints see the stream at all. |
| Active party without the responder's card | Nothing. Its first message fails and the responder closes without a reply. It cannot tell a Monolith service from anything else that closes a connection. |
| Active party with the responder's card | That the holder of the transport key is live at the address. No signature and nothing it could show to others. |
| Party that answers at the address without the transport key | 48 bytes it cannot use. The initiator stops at message 2 and has sent no identity. Obtaining the transport key later reveals nothing from what was recorded. |
| Authenticated initiator the responder does not hold as a contact | That its first message is answered with Close. Nothing about the contact list. |
| Authenticated responder | The initiator's card: identity, transport key, endpoints. |

Order. The responder is authenticated in message 2. The initiator reveals
its transport key and its identity in message 3, after that, with forward
secrecy. An initiator dials only identities it holds as contacts or has
asked to become contacts, and both receive its card in any case, so the
last row discloses nothing new.

A responder's handshake does not depend on what it holds about the
initiator. Its messages, its timing up to message 3 and its failures are
the same for a contact, a stranger and a blocked identity.

### 5.4 Informal security argument

- Impersonating the responder needs the responder's transport private key:
  message 2 is authenticated under a key that depends on it.
- Impersonating the initiator needs the transport private key of a card
  signed by the identity: message 3 is authenticated under a key that
  depends on it, and the card must name that transport key.
- Man in the middle. An attacker between A and B has to run its own
  handshake with each. Towards A it cannot produce message 2. Towards B it
  can only present a card of its own.
- Misbinding. Covered by F2 and F3: the holder of each transport key
  commits to the identity it acts as, inside the handshake.
- Replay. Each handshake uses new ephemeral keys on both sides. A replayed
  message 1 is answered with a message 2 that only the original initiator
  could use. A replayed message 3 does not fit another responder ephemeral
  key. A frame of another session does not decrypt.
- Reflection. Initiator and responder play different parts in the pattern,
  the prologue names the responder, and each side refuses its own
  identity.
- Key compromise impersonation. Someone who holds A's transport private
  key cannot pose as B towards A; Noise rates XK resistant from message 2
  on.
- Stale credentials. F4, within the limits stated in section 5.5.

### 5.5 What is not claimed

- Deniability. Monolith makes no deniability claim. A session contains no
  signature, but a contact card is a signed statement, and a peer that
  holds one can show it.
- Post-quantum security. X25519 and Ed25519 are not quantum-resistant.
  Recorded sessions could be decrypted by a future quantum computer. Tor's
  own onion service cryptography has the same limitation today. A hybrid
  handshake is a possible later protocol version; it is not part of
  version 1.
- Post-compromise security and revocation. A stolen identity key
  impersonates the identity until contacts are told out of band. A stolen
  transport key does the same towards any party that has not received a
  newer card. There is no automatic healing and no revocation mechanism in
  version 1.
- Binding to the onion address. A session is not tied to the endpoint that
  was dialed (PROTOCOL.md open question P2). A party that forwards bytes
  between an initiator and the real responder is not detected. It learns
  nothing and can change nothing.
- Protection against a compromised endpoint. Keys in the memory of a
  compromised machine are compromised.
- An audit. Noise XK has published analyses and `snow` was audited in
  2024. The five rules of section 5.2 and Monolith's use of the library
  have been reviewed by nobody outside the project.

## 6. Transport

- Each direction has its own cipher state from the Noise `Split()`.
- One frame is one Noise transport message: ChaCha20-Poly1305, 64-bit
  counter nonce starting at zero, empty associated data, a 16-byte tag.
- Frames are processed strictly in order on a reliable stream. A frame that
  fails authentication ends the session at once, so an attacker gets one
  forgery attempt per session.
- A frame that is replayed, dropped or reordered fails authentication,
  because the receiver's counter no longer matches. A frame of another
  session fails because the key differs.
- Length is the only thing visible outside the encryption, and it is
  quantized by padding (PROTOCOL.md section 5).
- Monolith adds no sequence numbers. Repetition at the level of messages
  is handled by the messages: a chat message that is sent again after a
  reconnect keeps its MessageId and is delivered once, and a contact card
  that is sent again is judged by its epoch.

## 7. Session lifetime and rekeying

A session ends, and a new handshake replaces it, when the first of these is
reached:

| Limit | Value | Counted |
| --- | --- | --- |
| Age | 24 hours | from the end of the handshake |
| Frames | 2^32 | per direction |
| Ciphertext bytes | 2^40 | per direction, the sum of the frame length fields |

The plaintext carried is always less than the ciphertext: 16 bytes less
per frame.

The cipher forces none of these. A Noise cipher state allows 2^64 - 1
messages, and integrity does not wear out with volume because one failed
frame ends the session. The limits bound how long one set of keys is in
use and make a long-lived connection take new ephemeral keys regularly.
The age limit is the one that is reached in practice. With the block size
of 1024 the byte limit is reached before the frame limit, at about 2^30
frames; the frame limit is the ceiling of the counter whatever the block
size is.

The age limit is deferred while a file transfer is active, so that the
clock does not cut a transfer off, but never beyond 48 hours in total. No
new transfer starts on a session that is past 24 hours.

Both sides count. A receiver that sees its peer exceed the frame or byte
limit treats that as a violation.

There is no in-band rekey. The Noise `Rekey()` function is not used. A
reconnect is a complete new handshake with new ephemeral keys, which is
simpler to reason about than a rekeyed session and gives new forward
secrecy. The side that reaches a limit sends Close and dials again;
unacknowledged messages are resent on the new session.

There is no session resumption, no pre-shared key and no data before the
handshake is complete.

The limits are constants, and tests run with reduced values to exercise the
path.

## 8. Forward secrecy and key lifetime

| Secret | Generated | Lifetime | Stored | Cloned | Erased |
| --- | --- | --- | --- | --- | --- |
| Identity private key | by Monolith | until the user discards the identity | vault, or memory in ephemeral mode | no | by its type on drop |
| Transport private key | by Monolith, independently | until replaced by a card with a greater epoch | vault, or memory in ephemeral mode | one copy per handshake, into the Noise state | by its type on drop; the copy when the handshake ends |
| Onion Service private key | by Tor or by Monolith | until the endpoint is retired | vault, or memory in ephemeral mode | no | by its type on drop |
| Noise ephemeral private key | per handshake | one handshake | never | no | when the handshake ends |
| Chaining key and handshake hash | by Noise | one handshake | never | no | not by `snow` |
| Frame cipher keys | by Noise | one session, at most 24 hours | never | no | when the session ends |
| Invitation capability | by Monolith | until revoked | vault | no | by its type on drop |
| Vault key | derived at unlock | while the vault is unlocked | never | no | by its type on drop |

Session keys and ephemeral keys are never written to storage. Types that
hold a secret do not derive `Debug`.

Compromise of the identity key, the transport key or the Onion Service key
does not reveal past sessions: after message 3 the keys depend on an
exchange between two ephemeral keys.

Erasure: Monolith's own secret types zeroize on drop. So do the objects
of the resolver that hold a key for `snow`: the copy of the transport
private key, the ephemeral private key and the cipher keys are held in
types that clear their memory when the handshake or the session is
dropped (ADR 0002, F-R1).

`snow` erases nothing it holds itself; that is an open finding of its 2024
audit. This leaves, uncleared, in memory that `snow` owns: the chaining
key of a handshake, the results of the Diffie-Hellman operations on its
stack, and the intermediate values of HMAC and HKDF, among them copies of
the cipher keys on their way into the resolver. Someone who can read the
freed memory of the process shortly after a handshake can find keys of
that session there. The transport private key is not among these values:
`snow` passes it to the resolver by reference and keeps no copy.

In no case does erasure reach copies the compiler made, freed memory that
was reused, pages that were swapped out, or crash dumps. This is listed as
a limit, not hidden.

Secrets are not locked into RAM. Doing so needs `mlock`, which needs
`unsafe` or a dependency that wraps it. On a system with swap, secrets can
reach the swap device. Tails has no swap. The recommendation for other
systems is encrypted swap or none; `monolith doctor` reports the swap
state.

## 9. Storage encryption

Specified in `STORAGE.md`. In short: Argon2id derives a key from the
passphrase, that key unwraps a random vault key, and the vault is encrypted
with XChaCha20-Poly1305 with the header as associated data. The vault
holds the identity private key, the transport private key and, where
Monolith manages them, the Onion Service private keys.

## 10. Implementation rules

- Noise is driven only through the library. No Monolith code implements a
  Noise step, a key derivation or a nonce.
- One crate holds every call to the Noise library. Contact logic, storage
  and front ends do not depend on it.
- The pattern string and the prologue label are constants, and a test
  asserts each.
- The handshake hash is read from the handshake state before it is
  converted to transport state.
- The only way to obtain a session that can send or receive a frame is a
  completed handshake. The object it returns owns the session logic of the
  protocol core; callers are not handed a session they could mark
  authenticated themselves.
- Ed25519 verification is always `verify_strict`. A lint bans the
  non-strict `verify` in Monolith crates.
- Test vectors with fixed keys are committed for every signed structure
  and for a complete handshake (PROTOCOL.md section 16). They were
  produced by an implementation that shares no code with Monolith, and
  the session crate has to reproduce them byte for byte.
- Every public key that enters a Diffie-Hellman operation passes one
  function in the resolver, which applies PROTOCOL.md section 10.2 and
  refuses an all-zero result.
- A handshake step consumes the object of the step before it. A step that
  fails leaves nothing to retry with and produces nothing to send.

## 11. Open points

The questions Q1 to Q11 of earlier drafts are closed by ADR 0002, and so
is the choice of the crypto provider (F-R1). What remains is listed
there:

- F-R2. The five rules have no external review.
- F-R3. `snow` has one maintainer.
- F-R4 and F-R5. No revocation of a transport key, and a stale card is
  accepted by a party that never saw a newer one.

PROTOCOL.md open questions P2 (binding to the onion key) and P8 (a period
in which two transport keys are answered) are open.

## 12. Sources

Accessed 2026-10-01.

- Noise Protocol Framework, revision 34: https://noiseprotocol.org/noise.html
  (5 processing rules, 7.7 payload security properties, 7.8 identity
  hiding, 12 and 14 on keys and application responsibilities)
- Noise Explorer: https://eprint.iacr.org/2018/766 ,
  https://noiseexplorer.com/patterns/XK/
- Flexible Authenticated and Confidential Channel Establishment:
  https://eprint.iacr.org/2019/436
- A Spectral Analysis of Noise:
  https://www.usenix.org/conference/usenixsecurity20/presentation/girol
- SIGMA: https://www.iacr.org/archive/crypto2003/27290399/27290399.pdf
- X25519, RFC 7748: https://www.rfc-editor.org/rfc/rfc7748
- ChaCha20 and Poly1305, RFC 8439: https://www.rfc-editor.org/rfc/rfc8439
- Ed25519, RFC 8032: https://www.rfc-editor.org/rfc/rfc8032
- Ed25519 strict verification:
  https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/struct.VerifyingKey.html
- Argon2, RFC 9106: https://www.rfc-editor.org/rfc/rfc9106
- XChaCha20-Poly1305:
  https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
- snow: https://github.com/mcginty/snow , its audit:
  https://github.com/trailofbits/publications/blob/master/reviews/2024-03-agilebits-snow-securityreview.pdf

The sources for the comparison of session protocols are in ADR 0002.
