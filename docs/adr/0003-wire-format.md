# ADR 0003: Wire format

Status: proposed, pending Phase 0 review
Date: 2026-10-01

## Context

The protocol needs an encoding for frames, messages and signed structures
that is binary, typed, versioned and bounded, and in which a signed
structure has exactly one byte representation. TorChat's newline-delimited
text with unbounded fields is the counterexample.

## Options

### A. Deterministic CBOR

RFC 8949 section 4.2 defines deterministic encoding: shortest-form
integers, no indefinite lengths, sorted map keys. It is compact and
extensible.

The Rust options do not enforce it on input. `minicbor` (maintained)
encodes shortest-form integers but its decoder accepts any width, has no
canonical mode, and leaves lengths and map order to the caller. `ciborium`
is deliberately liberal in what it accepts and has had no release since
January 2024. `serde_cbor` is unmaintained. Canonical input would have to be
enforced by Monolith, by decoding, re-encoding and comparing, or by a
strict decoder written by hand. Either is more code and more subtlety than
the data needs.

### B. Fixed layout

Fixed-width big-endian integers, fixed-size arrays, and length-prefixed
byte strings with a stated maximum, in a fixed field order. This is how TLS
structures, libp2p's signed prefix, minisign signatures and Tor cells are
laid out.

There is one encoding by construction. A decoder is a short sequence of
bounded reads. No generic serialization framework is involved, so nothing
depends on a serializer's behavior or version.

It is less flexible: adding a field means a new message type or a new
structure version.

### C. A serde-based format (bincode, postcard)

The byte layout would be a property of a library version and of derive
output. Unsuitable for anything that is signed or specified.

## Decision

Option B for everything on the wire and for the vault payload.

- Framing: fixed-size handshake records; then a 2-byte length and one
  encrypted frame. The length is checked against a state-dependent range
  before anything is read.
- The message type is inside the encrypted frame.
- Plaintext: type, body length, body, zero padding to a multiple of 1024
  bytes. Exactly minimal padding is required, so a body has one valid frame
  size.
- Bodies: fields in fixed order. Variable fields carry a 2-byte length and
  have a maximum in the specification. A body must be consumed exactly;
  trailing bytes are an error.
- Signed structures are verified over the received bytes.
- Extensibility: a major version in the preamble for incompatible changes;
  feature bits in AuthProof for compatible ones. A new message type may be
  sent only to a peer that advertised the feature. There are no ignorable
  types in version 1, so an unknown type is always an error.
- Text encoding of contact cards: a fixed prefix and unpadded base32, which
  is case-insensitive, safe to copy and compact in a QR code. The card's
  signature detects corruption.
- Encoders and decoders are written by hand in `monolith-protocol`, are
  sans-IO, and are covered by property tests and fuzz targets.

## Consequences

- The specification can state the exact size of every message, and does.
- Limits are enforced at the point of reading; there is no generic "read a
  value" call that could allocate from a declared length.
- More hand-written code than with a serialization library. It is simple,
  repetitive and testable.
- No dependency for serialization.
- Protocol evolution is deliberate. That is acceptable for a protocol that
  intends to stay small.

## Padding

Padding every frame to a multiple of 1024 bytes costs bandwidth that is
negligible for chat and under 0.1 percent for file transfer (chunks are
sized to fill a frame exactly). It hides the length of short messages from
anyone who sees ciphertext lengths between the application and Tor. It is
not presented as a defense against traffic analysis. The block size is an
open question (PROTOCOL.md P1).

## Sources

Accessed 2026-10-01.

- RFC 8949: https://www.rfc-editor.org/rfc/rfc8949
- minicbor: https://docs.rs/minicbor/2.3.0
- ciborium: https://crates.io/crates/ciborium
- https://rustsec.org/advisories/RUSTSEC-2021-0127.html
- RFC 8446, section 3 (presentation language) and 4.4.3:
  https://www.rfc-editor.org/rfc/rfc8446
- Noise specification, section 13 (application responsibilities):
  https://noiseprotocol.org/noise.html
