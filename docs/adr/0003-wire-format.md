# ADR 0003: Wire format

Status: accepted for Phase 1. The padding block size is provisional.
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

- Framing: a 2-byte length and one frame, inside the channel the session
  layer provides (ADR 0002). The length is checked against a
  state-dependent range before anything is read.
- The message type is inside the frame, and so inside the encryption.
- Plaintext: type, body length, body, zero padding to a multiple of 1024
  bytes. Exactly minimal padding is required, so a body has one valid frame
  size.
- Bodies: fields in fixed order. Variable fields carry a 2-byte length and
  have a maximum in the specification. A body must be consumed exactly;
  trailing bytes are an error.
- Signed structures are verified over the received bytes.
- Extensibility: the protocol identifier negotiated in the handshake
  names the version and any extension. A new message type may be sent only
  on a connection whose identifier includes it. There are no ignorable
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

The mechanism is decided: every frame plaintext is padded with zero bytes
to a multiple of a block size P, and exactly minimal padding is required.
The value of P is not decided. The specification and the code treat it as a
parameter, with 1024 as the working value.

What padding is for. It hides the exact length of a message inside its
bucket. It helps against an observer of the hop between the application
and Tor: loopback on most systems, the internal network on Whonix. Against
an observer of the Tor circuit it adds little below the size of a Tor cell,
because Tor already carries data in fixed-size cells of about 498 payload
bytes. Against correlation of traffic at both ends it does nothing. It is
not a defense against traffic analysis and must not be described as one.

What it costs, per frame, with a 2-byte length prefix and a 16-byte tag:

| P | Smallest frame on the stream | Tor cells for a short chat message | Lengths hidden |
| --- | --- | --- | --- |
| none | about 40 bytes | 1 | nothing |
| 256 | 274 | 1 | up to 252 bytes of body look alike |
| 480 | 498 | 1 | up to 476 bytes look alike; one frame fills one cell |
| 512 | 530 | 2 | up to 508 bytes look alike |
| 1024 | 1042 | 3 | up to 1020 bytes look alike |

- Amplification. A short message and its acknowledgement cost one cell each
  without padding and three cells each at P = 1024. For chat this is small
  in absolute terms and a factor of three in relative terms.
- Keepalives. One Ping and one Pong about every two minutes per session: at
  P = 1024 that is under 20 bytes per second per session.
- Fragmentation. Frames travel on a stream, so a frame that spans several
  cells needs no reassembly logic. More cells per message mean more
  exposure to cell-level timing, which argues for a block that fits one
  cell.
- File transfer. Chunks are sized to fill a maximum frame, so padding costs
  at most one block on the last chunk of a file, for any P.
- Unconfirmed sessions. The largest message before confirmation is 804
  bytes with its header. With P = 1024 every such frame is exactly one
  block, which gives the simple rule "exactly one block". A smaller P
  needs two or more blocks for that message and a slightly looser rule.

Candidates: 480 (one Tor cell per small frame), 512, 1024. The choice needs
a look at real message size distributions and at the cell counts on a
circuit, and is made before the protocol is frozen, not in Phase 1. Phase 1
implements the mechanism with P as a parameter.

## Sources

Accessed 2026-10-01.

- RFC 8949: https://www.rfc-editor.org/rfc/rfc8949
- minicbor: https://docs.rs/minicbor/2.3.0
- ciborium: https://crates.io/crates/ciborium
- https://rustsec.org/advisories/RUSTSEC-2021-0127.html
- RFC 8446, section 3 (presentation language) and 4.4.3:
  https://www.rfc-editor.org/rfc/rfc8446
