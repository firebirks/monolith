# ADR 0002: Session protocol

Status: proposed, pending Phase 0 review and review by a cryptographer
Date: 2026-10-01

## Context

Two peers connected over a Tor stream need mutual authentication of their
Monolith identities, a confidential channel with forward secrecy, and
resistance to replay and relay. Tor already encrypts Onion Service traffic
end to end; the session layer is defense in depth and is what ties a
connection to an identity that is independent of the onion address.

Requirements beyond the usual ones:

- nothing home-made: no custom key exchange, cipher or key schedule;
- the least possible novel composition;
- bounded, simple parsing in front of unauthenticated peers;
- no identity disclosure to a party that only knows the onion address;
- nothing that fingerprints the implementation or platform.

The starting proposal was Noise XX with BLAKE2s followed by a signed
identity proof, with TLS 1.3 to be evaluated as an alternative.

## Options

### A. Noise XX, then a signature over the handshake hash

Each side signs the Noise handshake hash with its identity key and sends
the signature inside the encrypted channel. The Noise specification
describes exactly this in section 11.2. Structurally it is the SIGMA
pattern, as in TLS 1.3 CertificateVerify.

### B. Noise XX with a signed static key in the handshake payload

What libp2p specifies: the identity key signs a prefix and the Noise static
public key; the signature travels in the payloads of messages 2 and 3. It
is widely deployed and can be copied verbatim.

Weaknesses for this project: the signature is a long-lived credential with
no freshness and no expiry, so a leaked static private key plus one
captured payload is a standing impersonation capability. The responder's
identity goes out in message 2 to anyone who connects.

### C. TLS 1.3 with mutual authentication and pinned raw public keys

`rustls` supports RFC 7250 raw public keys on both sides and Ed25519. With
the identity key as the TLS authentication key there is no composition to
design at all: the transcript signature is part of TLS and has been
analyzed extensively.

Costs:

- The default verifier does not accept self-managed keys. Both sides need
  custom verifiers written against the `danger` traits. That code is
  Monolith's and is as security-critical as an identity proof would be.
- The crypto provider is `ring` or `aws-lc-rs`, C and assembly. The pure
  Rust provider is experimental.
- A large state machine and parser face unauthenticated peers. Recent
  advisories in that surface include a panic in the acceptor and an
  infinite loop (RUSTSEC-2024-0399, RUSTSEC-2024-0336) and a handshake
  message boundary bug (RUSTSEC-2026-0285). All were fixed quickly; the
  point is the size of the surface.
- Handshake records are variable-length and the ClientHello is
  characteristic of the library and its version.
- The server sends its certificate to any client. There is no simple
  equivalent of binding the responder's identity into the handshake so that
  only parties who already know it get a proof. External PSKs could do it
  at the price of more configuration.
- Session resumption and tickets would have to be disabled and kept
  disabled.

The only public audit of `rustls` is from 2020.

### D. Noise with the identity key as the static key (XK or IK)

What Lightning and I2P do. Conflicts with ADR 0001: identity must be
separate from transport keys.

## Decision

Option A, with these parameters:

- `Noise_XX_25519_ChaChaPoly_SHA256`. SHA-256 instead of the proposed
  BLAKE2s: the Noise specification is neutral, this exact suite is the one
  libp2p deploys, and SHA-2 is already present through Ed25519.
- Prologue: the preamble and the responder's identity public key.
- Fresh static keys per connection, never stored.
- Empty handshake payloads, so the three messages are 32, 96 and 64 bytes.
- AuthProof as the first frame in each direction, responder first. The
  signed input is fixed-layout: prefix, role, version, handshake hash, both
  static keys, identity key, features (`CRYPTOGRAPHY.md` section 5).
- No in-band rekey. Sessions are bounded in time, frames and bytes and are
  replaced by a new handshake.
- Implementation: `snow` with its default pure-Rust resolver.

On whether to retain or replace the starting proposal: retain Noise XX with
a transcript-bound identity proof, with the changes above.

## Reasoning

On the amount of novel composition, TLS 1.3 with raw public keys is
strictly lower: zero. Option A has one small step that Monolith defines
itself. That step is the one the Noise specification describes, it mirrors
the structure of TLS 1.3's own authentication, and it is about ten lines of
specification.

TLS was not chosen because the requirements are not only about
composition:

- Parsing surface before authentication. Option A reads three fixed-size
  records and one fixed-size frame before the peer is authenticated. Every
  invariant about bounded input can be checked by inspection. TLS cannot
  offer that.
- No C or assembly in the session path, and a small dependency.
- Probe resistance through the prologue.
- No library fingerprint beyond "speaks Monolith version 1".
- The custom verifiers TLS would need are comparable in risk to the
  identity proof, so the practical difference in "code we must get right"
  is smaller than the theoretical one.

Option B was not chosen because of the standing credential and because the
responder's identity would go to any prober.

This is a judgment, and a reasonable reviewer could weigh audit history
higher. If review rejects option A, option C is the fallback, with
identities as raw public keys and resumption disabled.

## Consequences

- Connection setup takes two and a half round trips over an established
  circuit.
- The identity key signs once per session.
- The handshake costs five X25519 operations, a signature and a
  verification per side.
- Monolith owns the correctness of the AuthProof input and of the order
  rule "responder first, initiator only after verifying".

## Risks

- `snow` has had no formal audit, and its README says so. It is the
  de-facto standard Noise implementation in Rust and the backend of
  `libp2p-noise`.
- `snow` does not zeroize key material. Issues about this have been open
  since 2017. Monolith's own secrets are zeroized; session keys inside
  `snow` are not. Options: contribute zeroization upstream, wrap and
  minimize lifetimes, or evaluate `clatter`, which zeroizes but is less
  used and itself recommends `snow` for ordinary targets.
- `snow` releases are slow (0.10.0 is from July 2025) and it depends on the
  previous generation of the RustCrypto and dalek crates. Expect to align
  other crypto dependencies with it or carry duplicates.
- RUSTSEC-2024-0011 affected `snow` before 0.9.5. Require 0.9.5 or later.
- The handshake hash is available only before the transition to transport
  mode. The implementation must capture it at that point.

## Open questions

See `CRYPTOGRAPHY.md` section 11 (Q1 to Q6).

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
- CVE-2022-24759, signature validation failure in a libp2p Noise
  implementation: https://osv.dev/vulnerability/CVE-2022-24759
- Noise Explorer: https://eprint.iacr.org/2018/766
