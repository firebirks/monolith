# Monolith protocol, version 1

Status: the protocol core (framing, messages, text rules, contact cards,
session states, contact confirmation, duplicate resolution) is implemented
and accepted. The session layer, sections 3, 4 and 6.1 to 6.3, is the
design selected in ADR 0002: TLS 1.3 with raw public keys. It is specified
here and not implemented yet. Until it is, the code of the protocol core
still carries two things from the earlier draft that this document no
longer has: a message type for an identity proof, and a frame overhead of
16 bytes. Both go away with the implementation.

The padding block size in section 5 is a parameter whose production value
is not decided. Open questions are listed in section 17.

Related documents: `CRYPTOGRAPHY.md` (primitives, handshake analysis),
`RESOURCE_LIMITS.md` (every numeric limit), `SECURITY_INVARIANTS.md`.

## 1. Overview

Two peers talk over one Tor stream. The initiator opens the stream to the
responder's Onion Service; nothing in the protocol depends on Tor beyond a
reliable ordered byte stream.

A connection goes through these steps:

1. A TLS 1.3 handshake creates an encrypted channel and authenticates the
   Monolith identity of each side. The responder is authenticated first
   (sections 3, 4 and 6).
2. Messages are exchanged as padded frames inside that channel (section
   5), subject to the session state (section 7).

Design rules that hold throughout:

- A connection makes claims only about the identity of the peer that opened
  or accepted it. No message carries a third party's identity or address.
- Every record has a fixed size or a length prefix with a stated maximum.
  Lengths are checked before anything is read or allocated.
- Every structure has exactly one valid encoding.
- A violation of any rule in this document ends the session. The stream is
  closed without an error message; the peer is not told what was wrong.

## 2. Notation

- `u8`, `u16`, `u32`, `u64`: unsigned integers, big-endian.
- `[n]`: exactly n bytes.
- `bytes<a..b>`: a `u16` length L with a <= L <= b, followed by L bytes.
- `text<a..b>`: `bytes<a..b>` whose content is valid UTF-8 and passes the
  rules of section 9 for the field in question.
- `||`: concatenation.
- ASCII strings in double quotes are the bytes of the string without a
  terminator.

A message body is the concatenation of its fields in the order given. There
are no optional fields except where a presence byte is specified; a presence
byte is 0x00 or 0x01 and any other value is a violation. A body that is
longer or shorter than its fields require is a violation.

## 3. Protocol version

There is no preamble. The first bytes on the stream are a TLS 1.3
ClientHello.

The protocol version is selected with application-layer protocol
negotiation (ALPN, RFC 7301). This version of the protocol has the
identifier

    "monolith/1"      10 bytes: 6d 6f 6e 6f 6c 69 74 68 2f 31

- An initiator offers this identifier and no other.
- A responder that supports it selects it. A responder that supports none
  of the identifiers offered fails the handshake, as RFC 7301 requires.
- After the handshake each side checks that the negotiated identifier is
  `monolith/1`. A handshake that completed with another identifier, or
  with none, is a violation.

The offer and the selection are part of the handshake transcript, which
both sides sign, so neither can be changed without the handshake failing.
An initiator never retries with another version on its own; a failed
negotiation is reported to the user.

A later version of the protocol, or an optional extension of this one,
gets a new identifier. There are no feature bits.

The ClientHello shows that the endpoint speaks Monolith and which version.
It also shows what TLS library produced it. It contains no build, no
platform and no identity.

## 4. Handshake

The handshake is TLS 1.3 (RFC 8446). The initiator is the TLS client and
the responder is the TLS server. Each side authenticates with its Monolith
identity key, carried as a raw public key (RFC 7250).

### 4.1 Profile

| Parameter | Value |
| --- | --- |
| Version | TLS 1.3 only (0x0304) |
| Cipher suite | `TLS_CHACHA20_POLY1305_SHA256` (0x1303) |
| Key exchange group | x25519 (0x001D) |
| Signature scheme | ed25519 (0x0807) |
| Server certificate type (extension 20) | RawPublicKey (2) |
| Client certificate type (extension 19) | RawPublicKey (2) |
| Application protocol (extension 16) | `monolith/1` |
| Client authentication | required |
| Server name (extension 0) | not sent |
| Pre-shared keys, early data | not used |
| Session tickets | not sent by a responder, not stored by an initiator |
| Certificate compression | not used |

An implementation offers and accepts no other version, suite, group,
signature scheme or certificate type. X.509 certificates are not accepted.
An initiator sends its x25519 key share in the first ClientHello.

Extensions that a TLS library sends by default and that do not change any
parameter above are tolerated on receipt, as TLS requires.

Every connection is a full handshake. Nothing from one connection is
reused in another.

### 4.2 The identity key in the handshake

The Certificate message of each side carries exactly one entry, with no
extensions. The entry is the SubjectPublicKeyInfo of the sender's identity
key (RFC 8410), 44 bytes:

    30 2a                     SEQUENCE, 42 bytes
       30 05                  SEQUENCE, 5 bytes
          06 03 2b 65 70      OBJECT IDENTIFIER 1.3.101.112, Ed25519
       03 21 00               BIT STRING, 33 bytes, no unused bits
          key[32]             the identity public key

A receiver compares the first 12 bytes with this prefix and takes the
remaining 32 as the key. Anything else is rejected: another algorithm,
algorithm parameters, another length, trailing bytes, or more than one
entry. There is no ASN.1 parser in this path.

### 4.3 Signatures

Each side sends one CertificateVerify. It is an Ed25519 signature by the
identity key over these 130 bytes (RFC 8446 section 4.4.3):

| Size | Value |
| --- | --- |
| 64 | 0x20, repeated |
| 33 | "TLS 1.3, server CertificateVerify" from the responder, "TLS 1.3, client CertificateVerify" from the initiator |
| 1 | 0x00 |
| 32 | SHA-256 over all handshake messages so far, up to and including the signer's Certificate |

Monolith defines no signature of its own for the session.

### 4.4 Order and checks

    Initiator                                   Responder

      ClientHello                       ---->
                                                ServerHello
                                                {EncryptedExtensions}
                                                {CertificateRequest}
                                                {Certificate}
                                                {CertificateVerify}
                                        <----   {Finished}
      verifies the responder
      {Certificate}
      {CertificateVerify}
      {Finished}                        ---->
                                                verifies the initiator
      [frames]                          <--->   [frames]

Messages in braces are encrypted under handshake keys, frames under
application keys.

The initiator, on the responder's Certificate and CertificateVerify:

1. The entry has the form of section 4.2.
2. The key is a valid key (section 10.1).
3. The key is not the initiator's own identity key.
4. The key is byte for byte the identity that was dialed.
5. The signature of section 4.3 verifies under strict rules (section 10.1).

If any of these fails, the initiator ends the handshake. It has not sent
its own Certificate at that point, and it does not send it. A failure of
check 4 is reported to the user as an identity mismatch: the service at
the contact's address did not prove the contact's identity. This is never
resolved automatically, and there is no option to continue with the key
that was presented.

The responder, on the initiator's Certificate and CertificateVerify:

1. The entry has the form of section 4.2. An empty Certificate is a
   failure: client authentication is required.
2. The key is a valid key (section 10.1).
3. The key is not the responder's own identity key.
4. The signature of section 4.3 verifies under strict rules.

The responder checks nothing else here. In particular it does not look at
its contacts, its block list or any other record of the identity, so the
handshake behaves the same for every initiator with a valid key.

Both sides, after Finished:

1. The negotiated version is TLS 1.3 and the suite is 0x1303.
2. The negotiated application protocol is `monolith/1` (section 3).

The key that passed these checks is the peer's identity for the session.
It does not change for the lifetime of the session; TLS 1.3 has no
renegotiation, and post-handshake client authentication is not offered.

### 4.5 Bounds and failures

- A side that has received more than `MAX_HANDSHAKE_INPUT_LEN` (4096)
  bytes while the handshake is not complete closes the stream. The three
  flights of this profile are about 200, 370 and 190 bytes.
- The handshake must complete within `HANDSHAKE_TIMEOUT`.
- A handshake that fails ends with the stream closed. A TLS alert may be
  sent first. Every rejection that comes from the checks of section 4.4
  produces the same alert, `handshake_failure`, whichever check failed.
  Nothing specific to Monolith is sent.
- After the handshake, a violation of this document ends the session by
  closing the stream. No alert and no message is sent.

### 4.6 After the handshake

- A KeyUpdate from the peer is legal and is handled by the TLS layer. An
  implementation of this profile does not need to send one.
- A NewSessionTicket that is received is discarded.
- A side that ends a session on purpose sends the Close message (section
  8.1) and may follow it with a TLS `close_notify`. Receivers do not
  depend on `close_notify`: a stream that ends without the Close message
  is a transport failure whether or not it was sent.

## 5. Frames

After the handshake every transmission is a frame:

    u16 length || plaintext[length]

`length` must satisfy

    1024 <= length <= 64512  and  length mod 1024 == 0

and, while the session is in `AuthenticatedUnknown`, `length` must be
exactly 1024. No frame is accepted before that state. A length that fails
these checks is a violation and is detected before the frame is read.

Frames are written to the TLS stream as application data. TLS protects
them; a frame is not encrypted a second time. A frame may be split across
TLS records, and record boundaries mean nothing to this layer. Frames
cannot be reordered, dropped or replayed without the TLS layer failing,
which ends the session.

The plaintext is:

    u16 type || u16 body_length || body[body_length] || padding

- `padding` is zero bytes up to the next multiple of 1024. The plaintext
  length must equal `4 + body_length` rounded up to a multiple of 1024; more
  padding than necessary, or a non-zero padding byte, is a violation.
- `type` must be an assigned code (section 8). An unassigned code is a
  violation. There are no ignorable message types in version 1; a type that
  is added later may be sent only on a connection whose protocol
  identifier (section 3) includes it.
- The message must be legal in the current session state (section 7). This
  is checked before the body is parsed.

One frame carries one message. No message spans frames.

### 5.1 Frame parameters

Two numbers in this section are parameters, not constants of the design:

- P, the padding block. This document uses P = 1024. That value is
  provisional; ADR 0003 gives the trade-off and the alternatives.
- T, the number of bytes the session layer adds to each frame. With TLS
  underneath, the stream is already protected and T = 0. A session layer
  that encrypted each frame itself would add its authentication tag here.

In terms of them:

- the plaintext length is a multiple of P, at least P, and at most the
  largest multiple of P that keeps `length` within 65535;
- `length` is the plaintext length plus T;
- before a session is confirmed, the plaintext is no longer than the padded
  size of the largest message that is legal then. That message is a
  ContactRequest of 800 bytes, 804 with its header. For P = 1024 this is
  one block, which is where "exactly 1024" comes from.

A pair of values is usable only if P is larger than the 4-byte message
header, P + T is at most 65535, and the largest plaintext can hold every
message other than a FileChunk at its largest size. The largest of those is
a ChatMessage of 16402 bytes. An implementation refuses any other pair.

A FileChunk is cut to the frame: a full chunk carries as much data as the
largest body holds after the 16-byte transfer identifier and the 2-byte
length, and never more than 64490 bytes.

The limits 1024, 64512, 64508 and 64490 elsewhere in this document are
these rules evaluated for P = 1024 and T = 0. An implementation takes
P and T as parameters, so that changing either is a change of two numbers
and not of the frame decoder.

Padding hides the exact length of short messages from anyone who can see
record lengths on the path between the application and Tor. It is not a
defense against traffic analysis; see `THREAT_MODEL.md`.

## 6. Identity authentication

### 6.1 Authentication by the handshake

The identity of each side is authenticated by the TLS handshake of section
4. There is no separate identity proof and no message for one. Message
code 0x0001, which earlier drafts gave to such a message, is not assigned.

### 6.2 Result

When the handshake and the checks of section 4.4 have succeeded, both
sides are in `AuthenticatedUnknown`: the peer's identity is proven, and
whether the two are contacts has not yet been confirmed on this session.
Section 6.4 says how a session leaves that state.

The responder was authenticated first. The initiator's identity was sent
only to a responder that had already proved the identity that was dialed.

Authentication says who the peer is. It does not say what the peer may
do. A peer with a valid key that the local side has never seen, has
declined, has blocked or has deleted is authenticated exactly like a
contact, and is then handled by sections 6.4 and 12.

Budgets are applied at this point according to the local record of the
proven identity. A session with an identity the responder holds as an
accepted or requested contact counts against `MAX_CONTACT_SESSIONS`. Every
other session counts against `MAX_UNKNOWN_SESSIONS` and
`UNKNOWN_SESSION_RATE`; if the rate is exhausted, the responder closes.
Holders of a contact card who are not contacts therefore cannot use up the
room that contacts need. They can use up the room for other strangers; see
`RESOURCE_LIMITS.md` section 12.

No other check is applied here. In particular a blocked identity is not
turned away at this point; it is handled like any other identity that is
not a contact (section 12), so that being blocked cannot be told from being
unknown.

### 6.3 Extensions

Version 1 has no optional extensions and no negotiation beyond the
protocol identifier of section 3. An extension that is added later is a
new identifier. Identifiers describe protocol behavior only and are never
used to convey platform, build or product information.

### 6.4 Contact confirmation

What a side sends first in `AuthenticatedUnknown` depends only on what it
holds locally about the peer's identity:

| Local record of the peer | First message |
| --- | --- |
| accepted contact | ContactAccept |
| requested (the user imported the peer's card; no acceptance seen yet) | ContactRequest |
| none, declined or blocked | nothing |

A session becomes `AuthenticatedContact` on a side when both of these are
true: the side holds the peer as an accepted contact, and it has received
ContactAccept from the peer on this session. A side sends no message other
than ContactAccept, ContactRequest and Close before that.

This is repeated on every session and costs one frame in each direction.
Its purpose is that no contact-only message is ever sent to a peer that
would reject it. If one side deleted the other, lost its state or restored
an old backup, the session does not get confirmed. The protocol does not
repair the disagreement itself: the side that still holds the contact keeps
offering ContactAccept and sees the contact as not confirmed until one of
the two users sends a new request.

Receiving, in `AuthenticatedUnknown`:

- ContactAccept from a peer held as accepted: confirmed, as above.
- ContactAccept from a peer held as requested: the peer accepted. Mark it
  accepted, send ContactAccept, and the session is confirmed.
- ContactRequest from a peer held as requested: both sides asked. Mark it
  accepted and send ContactAccept. The session is confirmed when the peer's
  ContactAccept arrives.
- ContactRequest from a peer held as accepted: the peer does not know yet,
  or has lost its record. ContactAccept was already sent; nothing more to
  do.
- Anything from a peer with no record, declined or blocked: section 12.

Before any of this, a ContactRequest is checked against the proven identity
(section 8.3). That check does not depend on the record of the peer.

A peer sends at most one ContactRequest on a session. A second one before
the session is confirmed is a protocol violation.

A session that is not confirmed within `UNKNOWN_SESSION_TIMEOUT` is closed.

## 7. Session states

    Connecting -> CryptoHandshake -> IdentityAuth -> AuthenticatedUnknown
    AuthenticatedUnknown -> AuthenticatedContact
    any state -> Closed
    AuthenticatedUnknown, AuthenticatedContact -> Closing -> Closed

| State | Meaning | Messages accepted from the peer |
| --- | --- | --- |
| Connecting | Stream being opened | none |
| CryptoHandshake | TLS handshake in progress | none (TLS handshake messages only) |
| IdentityAuth | TLS handshake complete; the checks of section 4.4 are applied to its result | none |
| AuthenticatedUnknown | Peer's identity proven; contact relationship not confirmed on this session | ContactRequest, ContactAccept, Close |
| AuthenticatedContact | Both sides hold each other as accepted contacts and have said so on this session | every message type |
| Closing | Close sent; the stream is being shut down | none |
| Closed | Terminal | none |

A session never moves backwards, and every session passes through
`AuthenticatedUnknown`. The implementation of this table is
`MessageType::may_be_received_in` and `SessionState::can_transition_to` in
`monolith-protocol`; the logic of sections 6.4 and 12 is `session::Session`.

A session enters `AuthenticatedUnknown` only for the identity whose key
the handshake authenticated. The session logic is given that identity by
the handshake and by nothing else. It ends the session when that identity
is the local one, and, on a session the local side opened, when it is any
identity other than the one that was dialed (section 4.4). An
implementation does not let a caller move a session into
`AuthenticatedUnknown` by any other route.

A Close that is received ends the session at once: the receiver goes to
`Closed`. After a Close was sent or received, a side stops reading from the
stream. Bytes still in flight are dropped without being decoded.

A session leaves `AuthenticatedUnknown` within `UNKNOWN_SESSION_TIMEOUT`,
by confirmation or by closing.

## 8. Messages

All sizes are body sizes. "States" lists where the message may be received.

| Code | Name | Body size | States |
| --- | --- | --- | --- |
| 0x0002 | Close | 0 | Unknown, Contact |
| 0x0003 | Ping | 8 | Contact |
| 0x0004 | Pong | 8 | Contact |
| 0x0010 | ContactRequest | 144 to 800 | Unknown, Contact |
| 0x0011 | ContactAccept | 0 | Unknown, Contact |
| 0x0020 | ChatMessage | 19 to 16402 | Contact |
| 0x0021 | MessageAck | 16 | Contact |
| 0x0030 | Profile | 4 to 1156 | Contact |
| 0x0031 | EndpointUpdate | 139 | Contact |
| 0x0040 | FileOffer | 27 to 281 | Contact |
| 0x0041 | FileAccept | 16 | Contact |
| 0x0042 | FileReject | 16 | Contact |
| 0x0043 | FileChunk | 19 to 64508 | Contact |
| 0x0044 | FileComplete | 48 | Contact |
| 0x0045 | FileAbort | 16 | Contact |

Code 0x0001 is not assigned.

No message has a field that names a third party. The only identities and
endpoints on the wire are the sender's own, inside structures the sender
signed.

### 8.1 Close (0x0002)

Empty body. The sender will send nothing more and closes the stream after
writing the frame. The receiver stops processing and closes. Close carries
no reason. A stream that ends without Close is treated as a failure of the
transport; the effect on queued messages and transfers is the same.

Close is sent whenever a side ends a session on purpose: the user quits, a
session limit is reached, a duplicate session is resolved, or an
unconfirmed session is turned away (section 12). After a protocol violation
the stream is closed without it. Frames that arrive after Close was sent or
received are discarded without being processed.

### 8.2 Ping (0x0003) and Pong (0x0004)

    [8] nonce

A side that has received no frame for a randomized interval between
`PING_INTERVAL_MIN` and `PING_INTERVAL_MAX` sends a Ping with a random
nonce. The receiver answers with a Pong carrying the same nonce. A Pong
that does not match an outstanding Ping is a violation. A Ping that is not
answered within `PONG_TIMEOUT` ends the session.

### 8.3 ContactRequest (0x0010)

    [139]        card           the sender's contact card, no capability
    u8           has_capability 0x00 or 0x01
    [16]         capability     present only if has_capability is 0x01
    text<0..128> display_name
    text<0..512> introduction

`card` must be a valid contact card (section 11) whose identity key equals
the identity the sender proved in the handshake, and it must not carry a
capability of its own. A request with the card of another identity is a
protocol violation: the stream is closed and nothing is sent. The receiver
makes this check before it looks at what it holds about the sender, so the
outcome is the same for a stranger, a blocked identity and a contact.

`capability` is the invitation capability copied from the card of the peer
being asked (section 12).

Receiver behavior is in sections 6.4 and 12. The receiver's response to a
well-formed request from an identity it neither requested nor accepted is
always the same: it sends Close.

Received in `AuthenticatedContact`, a ContactRequest is validated in the
same way and then ignored.

### 8.4 ContactAccept (0x0011)

Empty body. Tells the peer that the sender holds it as an accepted contact.
Section 6.4 defines when it is sent and what it does. From an identity the
receiver neither requested nor accepted it has no effect and is answered
like a request that was dropped: with Close.

Received in `AuthenticatedContact`, a ContactAccept is ignored.

### 8.5 ChatMessage (0x0020)

    [16]           message_id
    text<1..16384> text

`message_id` is 16 bytes from the CSPRNG, chosen by the sender. It carries no
time or counter.

The receiver looks the identifier up in the per-contact window of the last
`MESSAGE_DEDUP_WINDOW` received identifiers. If it is present, the receiver
sends MessageAck again and does nothing else. If not, it hands the message
to the application (and to the history store, if enabled), records the
identifier, and then sends MessageAck.

The window is kept in memory, and in the message store when history is
enabled. With history off, a restart of the receiver empties it, and a
message that the sender resends afterwards is delivered a second time.

No timestamp is transmitted. Each side records its own local time of
sending and receiving if it wants one.

### 8.6 MessageAck (0x0021)

    [16] message_id

The message with this identifier was received, validated and handed to the
application or the store. It says nothing about the message having been
displayed or read, or about a person being present.

The sender keeps a message queued until it is acknowledged and resends
unacknowledged messages, in their original order, on the next session. An
acknowledgement for an unknown identifier is ignored.

### 8.7 Profile (0x0030)

    text<0..128>  display_name
    text<0..1024> profile_text

Replaces the profile the receiver holds for this contact. Sent only to
accepted contacts: once after a session is confirmed, and again when the
sender's profile changes. The display name is a suggestion; it never
replaces the fingerprint and never overrides a name the local user
assigned. A Profile equal to the stored one causes no event.

### 8.8 EndpointUpdate (0x0031)

    [139] card   the sender's contact card, no capability

`card` must be a valid contact card whose identity key equals the sender's
proven identity; a card of another identity is a protocol violation. The
receiver compares it with what it has pinned for this contact:

- greater epoch: recorded as a pending endpoint change. In version 1 the
  change takes effect after the user confirms it;
- same epoch and the same endpoint set: nothing to do. This is the normal
  case;
- same epoch and a different endpoint set: the owner signed two statements
  with one epoch. Ignored, and reported to the user as an anomaly;
- lower epoch: ignored and counted.

Nothing but a greater epoch ever changes the pinned endpoint.

Each side sends its current card once after a session is confirmed, and
again if its endpoint changes during the session. Sending it every time
keeps the sender from having to track what each contact already knows, and
it is how a contact learns a new endpoint: the owner dials out from the new
endpoint and says so.

### 8.9 File transfer (0x0040 to 0x0045)

    FileOffer     [16] transfer_id || u64 size || text<1..255> filename
    FileAccept    [16] transfer_id
    FileReject    [16] transfer_id
    FileChunk     [16] transfer_id || bytes<1..64490> data
    FileComplete  [16] transfer_id || [32] sha256
    FileAbort     [16] transfer_id

Section 13 defines the exchange.

## 9. Text rules

All text fields must be valid UTF-8. Every rule in this section is a
length in bytes, a count of scalar values, or membership in a list of code
points that is written out here. None of them refers to Unicode tables, so
the outcome does not depend on the Unicode version an implementation was
built with. Text that passes is kept and delivered byte for byte as it was
sent. No normalization form is required, and none is applied to anything
that is signed, sent or compared.

In addition:

Every text field rejects:

- U+0000 to U+0008, U+000B, U+000C, U+000E to U+001F, U+007F (C0 controls
  and DEL, except those allowed below);
- U+0080 to U+009F (C1 controls);
- U+2028 and U+2029 (line and paragraph separator);
- noncharacters (U+FDD0 to U+FDEF and the last two code points of each
  plane).

`ChatMessage.text`, `ContactRequest.introduction`, `Profile.profile_text`:

- U+0009 (tab) and U+000A (line feed) are allowed. U+000D is rejected;
  senders convert line endings to U+000A.
- No normalization is applied and none is required. The text is delivered
  as sent.
- Bidirectional and zero-width format characters are allowed, because
  right-to-left scripts and some emoji sequences need them. Containing their
  effect is the renderer's job (section 9.1).

`display_name` and `filename`:

- U+0009, U+000A and U+000D are rejected.
- Bidirectional controls are rejected: U+061C, U+200E, U+200F, U+202A to
  U+202E, U+2066 to U+2069.
- Code points with the Unicode property Default_Ignorable_Code_Point are
  rejected, except variation selectors. As of Unicode 16 that is, next to
  the bidirectional controls above: U+00AD, U+034F, U+115F, U+1160, U+17B4,
  U+17B5, U+180E, U+200B to U+200D, U+2060 to U+2065, U+206A to U+206F,
  U+3164, U+FEFF, U+FFA0, U+FFF0 to U+FFF8, U+1BCA0 to U+1BCA3, U+1D173 to
  U+1D17A, U+E0000 to U+E00FF, U+E01F0 to U+E0FFF. The list is fixed in
  this document; it does not change with the Unicode tables of a build.
- Variation selectors are allowed: U+180B to U+180D, U+180F, U+FE00 to
  U+FE0F, U+E0100 to U+E01EF. Emoji and some scripts need them, and they
  do not hide text.
- A few characters that are not ignorable by default but are drawn as
  nothing or as a blank are rejected: U+2800 (blank braille pattern),
  U+FFF9 to U+FFFB (interlinear annotation), U+FFFC (object replacement
  character).
- These rules remove the known ways to hide characters in a name. They do
  not make two names that look the same be the same; section 10 is what
  identifies a contact.
- U+0020 is the only whitespace character allowed. The others are
  rejected: U+00A0, U+1680, U+2000 to U+200A, U+202F, U+205F, U+3000, next
  to U+0085, U+2028 and U+2029, which every field rejects. The text must
  not begin or end with U+0020.
- `display_name`: at most 64 scalar values, counted in the text as sent.
  A name does not have to be in any normalization form. Two names that
  are drawn alike may differ in their bytes, and both are valid.
- `filename`: additionally rejects `/`, `\`, and the names `.` and `..`.
  Section 13.3 lists what the receiver does before it uses the name locally.

A sender applies the same rules to its own input, so a conforming peer never
triggers them.

A display name is not a security identity. It is never used for identity,
authentication, contact equality, authorization, duplicate detection or
protocol state; those use the identity key (section 10). A front end may
normalize a copy of a name for drawing or for searching. That copy is never
signed, sent or used in a decision of the protocol.

### 9.1 Rendering

Protocol validation does not make text safe to draw. A front end must also:

- draw text with a plain-text widget; never interpret markup;
- isolate each peer-supplied string from surrounding UI text for
  bidirectional layout, so that its direction cannot spill over;
- apply the rendering limits in `RESOURCE_LIMITS.md` section 10;
- escape control and format characters when writing to a terminal;
- never treat a URL in text as anything but text.

## 10. Identity fingerprint

    fingerprint = SHA-256("MONOLITH-FINGERPRINT-V1" || 0x01 || identity_public_key)

The byte 0x01 names the key type (Ed25519).

- Full form: the 32 bytes in base32 (RFC 4648 alphabet, upper case, no
  padding), 52 characters, shown in 13 groups of 4.
- Compact form: the first 15 bytes in base32, 24 characters, shown in 6
  groups of 4. It has 120 bits and is used where space is short. The full
  form is always available next to it in verification screens.

Fingerprints are compared out of band. A display name is never a substitute.

### 10.1 Valid keys and signatures

A 32-byte string is a valid Monolith Ed25519 public key only when all of
these hold:

1. it is a compressed Edwards encoding that decompresses to a point of the
   curve;
2. the point is torsion-free: multiplying it by the order of the
   prime-order subgroup gives the identity element;
3. the point is not of small order.

Neither of the last two is sufficient alone. The identity element is
torsion-free, and only condition 3 rejects it. A point with a torsion
component that is not itself of small order passes condition 3, and only
condition 2 rejects it. The seven other points of small order fail both.
Together the conditions say that the key is a point of the prime-order
subgroup other than the identity element, which is what every honestly
generated key is.

A valid key has exactly one encoding: every non-canonical encoding of a
point of this curve is a point of small order or one with a torsion
component, so the conditions reject it.

This is deliberately stricter than Ed25519 signature verification, which
accepts every point that decompresses. Monolith generates its identity
keys itself and does not need to accept identity keys made by other
software.

The same rule is used for identity keys and for the Ed25519 master public
key of an Onion Service, as it appears in a contact card and in a v3 onion
address. Condition 2 is the test Tor applies to that key.

Honest key generation never produces a point with a torsion component. If
such points were accepted, the holder of one secret could present up to
eight public keys, the honest one plus each point of small order, and
produce signatures that verify under each of them: with a few attempts
under strict verification, and with none under rules that multiply by the
cofactor. Refusing them keeps one secret from standing behind several
pinned identities, whatever verification rules a later implementation
uses.

A signature is valid if it verifies under the strict rules of the Ed25519
implementation: the scalar is canonical and the R component is not of small
order.

## 11. Contact card

A contact card is a signed statement by an identity: "as of this epoch, I
can be reached at this set of endpoints".

### 11.1 Binary form

With n = `endpoint_count`:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | version = 0x01 |
| 1 | 32 | identity_public_key |
| 33 | 8 | endpoint_epoch (`u64`, at least 1) |
| 41 | 1 | endpoint_count (n) |
| 42 | 32 x n | endpoints: one onion_service_key each |
| 42 + 32n | 1 | flags |
| 43 + 32n | 0 or 16 | invitation_capability |
| then | 64 | signature |

`endpoint_count` is at least 1 and at most `MAX_ACTIVE_ENDPOINTS`. In
version 1 that maximum is 1, so every valid card has exactly one endpoint
and is 139 bytes long without a capability, 155 with one. The endpoints in
a card are distinct.

`flags`: bit 0 set means `invitation_capability` is present. Bits 1 to 7 are
zero; a card with any of them set is invalid.

An `onion_service_key` is the 32-byte Ed25519 public key of an Onion
Service v3. The address is derived from it as the Tor specification
defines:

    checksum = SHA3-256(".onion checksum" || onion_service_key || 0x03)[0..2]
    address  = base32(onion_service_key || checksum || 0x03) || ".onion"

Storing the key instead of the address means a card cannot carry a wrong
checksum or version byte. Only version 3 services can be expressed.

### 11.1.1 Signed bytes

`signature` is an Ed25519 signature by `identity_public_key` over the
signed bytes of the card. The signed bytes are defined here, field by
field, and not by reference to the transport layout:

    "MONOLITH-CONTACT-CARD-V1"   24 bytes
    version                       1 byte
    identity_public_key          32 bytes
    endpoint_epoch                8 bytes, big-endian
    endpoint_count                1 byte
    endpoints                    32 bytes each, in card order
    flags                         1 byte
    invitation_capability         0 or 16 bytes

In version 1 these are the bytes of the binary form up to the signature,
after the prefix. An implementation still builds them with a function of
its own, separate from the transport encoder, so that a later change to
the transport layout cannot change what a signature means. With one
endpoint the signed bytes are 99 or 115 bytes long.

### 11.2 Validation

In this order; the first failure rejects the card:

1. Length is at least the size of the fixed fields.
2. `version` is 0x01.
3. `endpoint_epoch` is at least 1.
4. `endpoint_count` is between 1 and `MAX_ACTIVE_ENDPOINTS`.
5. `flags` has no reserved bit set.
6. The length is exactly what `endpoint_count` and `flags` imply. There are
   no trailing bytes.
7. `identity_public_key` is a valid identity key (section 10.1).
8. Every endpoint is a valid onion service key (section 10.1), no two are
   equal, and none is byte for byte equal to `identity_public_key`. The
   last part is key separation; see below.
9. The signature is valid (section 10.1) over the signed bytes built from
   the decoded fields.

The decoder accepts exactly one encoding of a card, so the signed bytes
built from the decoded fields are determined by the received bytes and by
nothing else.

Key separation. The Monolith identity key and the master key of a Tor
Onion Service belong to different cryptographic domains. The first signs
contact cards under Monolith's prefix and TLS handshake transcripts
under the context strings of TLS; the second is
used by Tor, under Tor's rules, to certify the keys of a service. A key
must never be used in both. A card in which an endpoint is the identity
key states that it is, so the card is invalid, and an implementation
refuses to sign one. This is an invariant of the protocol and not a
side effect of validation: it is S33 in `SECURITY_INVARIANTS.md`.

The rule catches a key that is reused on purpose or by a bug in key
handling. It cannot catch two keys derived from one secret by different
means; `CRYPTOGRAPHY.md` section 3 requires the two keys to be generated
independently.

### 11.3 Text form

    "MONOLITH1:" || base32(card)

Base32 uses the RFC 4648 alphabet without padding. The canonical form is
upper case, which lets a QR code use alphanumeric mode. The longest card in
version 1 is 258 characters.

Parsing: input longer than `MAX_CONTACT_CARD_TEXT_LEN` bytes is
rejected. ASCII space, tab, CR and LF are removed wherever they occur.
Letters are accepted in either case. Any other character, a wrong prefix, or
unused trailing bits that are not zero reject the input. The decoded bytes
are then validated as in 11.2.

The "1" in the prefix is the version of the text encoding. A corrupted card
fails signature verification; no separate checksum is needed.

A contact card is sensitive: whoever holds it can try to connect to the
user's Onion Service, and can tell from the attempt whether that endpoint
is reachable at the moment. Monolith does not hide endpoint availability
from anyone who holds the card. Cards and QR codes are produced locally and
are never uploaded anywhere.

### 11.4 Endpoint sets and epochs

An identity has a set of endpoints, not one endpoint. The model is

    identity -> endpoint set, as of an epoch

and version 1 limits the set to one member (`MAX_ACTIVE_ENDPOINTS` = 1).
The count field, the signed bytes and the epoch rule are already those of a
set, so that these can be supported later by raising the maximum, without
a new identity model or a new signature format:

- rotation, with the old and the new endpoint both valid for a while;
- endpoints used during a migration;
- temporary endpoints, which a later statement drops again;
- endpoints given to one contact only. This needs no extra field: epochs
  only have to increase as seen by each receiver, so an identity can sign
  different statements for different contacts.

None of these is implemented in version 1, and a version 1 card with more
than one endpoint is invalid.

The owner of an identity keeps a counter. Each time its endpoint set changes
it increments the counter and signs a new card. A card always states the
whole set; there is no "add" or "remove".

A receiver pins, per contact, the identity key, the endpoint set and the
epoch. A card signed by the pinned identity replaces the pinned set only if
its epoch is strictly greater, and only when it arrives

- in an EndpointUpdate on an authenticated session with that contact, or
- as a card the user imports by hand.

A card with a different identity key is a different contact, whatever
display name comes with it.

If every endpoint a contact knows has disappeared before an update reached
it, there is no way for the contact to learn the new endpoint from the
network. The user has to hand over a new card out of band.

## 12. Contact requests and invitations

An invitation capability is 16 random bytes that the user's card may carry.
It is an anti-spam token. It is not an identity and it authenticates nobody.
Its size is fixed. Capabilities are compared without an early exit on the
first difference. That is a best-effort property of the implementation,
not a claim that no timing side channel exists on any hardware; and
section 12.1 already excludes response times from what the protocol
promises.

Local policy has three modes:

- invitation (default): a request is surfaced only if it carries a
  capability that is currently valid;
- open: a request is surfaced with or without a capability;
- closed: no request is surfaced.

A user can hold up to `MAX_ACTIVE_INVITATIONS` valid capabilities and revoke
any of them. Revoking a capability does not change the identity or the
endpoint; it only stops cards that carry it from producing requests.

Processing of the first message received in `AuthenticatedUnknown` from a
peer that the local side neither requested nor accepted. Peers that are
requested or accepted are handled by section 6.4.

1. Validate the body, including that the card in a ContactRequest is the
   sender's own (section 8.3). A malformed message is a violation.
2. Send Close. This happens whether a request will be queued or dropped and
   whatever the reason, so the sender cannot tell the cases apart. A
   blocked sender sees exactly what a stranger with a bad invitation sees.
3. If the message is a ContactRequest, decide whether to queue it. The
   request is dropped if the sender is on the block list or the declined
   list, if the policy mode does not admit it, if the capability does not
   match a valid one (compared in constant time), if a request from this
   identity is already pending, if `MAX_PENDING_REQUESTS_PER_INVITATION`
   requests with the same capability (or, in open mode, with none) are
   already pending, or if the queue holds `MAX_PENDING_CONTACT_REQUESTS`
   entries. A ContactAccept is dropped.

The Close is written and the stream closed before the decision is taken,
so that the work of taking it happens after the last thing the sender can
observe.

If no message arrives within `UNKNOWN_FIRST_MESSAGE_TIMEOUT`, the session
is closed. When `MAX_UNKNOWN_SESSIONS` such sessions exist and another peer
authenticates, the oldest one that has not yet sent its message is closed
to make room.

The quota per capability keeps one leaked invitation from filling the whole
queue: its holder can occupy at most
`MAX_PENDING_REQUESTS_PER_INVITATION` entries until the user revokes it.

A queued request holds the sender's identity, card, display name and
introduction. Nothing is written to the contact store. The user then
accepts, declines or blocks:

- accept: a contact is created from the card in the request, with the
  identity pinned. The next session with that identity, in either direction,
  delivers ContactAccept (section 6.4).
- decline: the identity goes on the declined list and later requests from
  it are dropped. Nothing is sent. The list holds
  `MAX_DECLINED_IDENTITIES` entries; the oldest is dropped when it is full.
- block: the identity goes on the block list. Nothing is sent. Blocking an
  accepted contact also ends any session with it and removes it from the
  set of accepted contacts for the purpose of section 6.4.

The requester learns of an acceptance when a later session delivers
ContactAccept. It never learns of a decline or a block; from its side they
look like a request that is still pending.

The requesting side retries on the normal reconnect schedule and sends its
ContactRequest again on each session until it receives ContactAccept.

A peer that was deleted or blocked by the other side keeps sending
ContactAccept at the start of each session and never gets one back. Its
interface shows the contact as not confirmed, which is the same thing it
shows for a contact that is offline or has not accepted yet.

Before acceptance the requester receives no profile, no presence beyond the
fact that the Onion Service answered, and nothing about other contacts.

### 12.1 What a peer can observe about its standing

A peer must not be able to find out, from how Monolith answers it, whether
it is unknown, blocked, declined or a contact that was deleted. The table
gives what the local side does toward a peer that connected and proved an
identity, by the local record of that identity.

| Local record of the peer | Sent before the peer's first message | Answer to its ContactRequest | Answer to its ContactAccept | If it stays silent | Profile, endpoint update, application messages |
| --- | --- | --- | --- | --- | --- |
| none (never seen) | nothing | Close | Close | Close at `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | never |
| blocked | nothing | Close | Close | Close at `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | never |
| deleted former contact | nothing | Close | Close | Close at `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | never |
| declined | nothing | Close | Close | Close at `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | never |
| requested by the local user | ContactRequest | ContactAccept | ContactAccept | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |
| accepted, verified out of band | ContactAccept | nothing more | confirmed | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |
| accepted, not verified | ContactAccept | nothing more | confirmed | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |

For every row: the handshake is the same, a
protocol violation ends the stream with nothing sent, and Close has an
empty body.

Requirements that follow:

- The first four rows are the same row. Message types, their number and
  order, and the conditions under which the session is closed must not
  depend on which of the four applies. The implementation takes one code
  path for them, and the record is consulted only to decide whether a
  request is put in the queue, which the peer cannot see.
- The last two rows are the same row. Whether the user has verified a
  contact is local information; no function that produces protocol output
  takes it as input.
- There is no message, field or code that states a reason. Nothing in the
  protocol says "blocked", "former contact" or "not in the contact list",
  and no such thing may be added for peers that are not confirmed contacts.
- An accepted contact that is blocked or deleted while a session is open
  sees Close, as for any other end of a session, and from then on the
  first row.
- A ContactRequest that carries the card of another identity is a protocol
  violation in every row.

What a peer can still learn, by design:

- An accepted peer learns that it is accepted, and a requested peer learns
  that it was requested. That is the purpose of those messages.
- A peer that was a contact and no longer gets ContactAccept knows that it
  is not confirmed. It cannot tell deletion from blocking, from a restored
  backup, or from the other side having lost its data.
- When the budget for strangers is exhausted, the first four rows are closed
  right after authentication while the last three are not. This separates
  the same two groups that the messages already separate.

Timing is not part of this guarantee. The same steps are taken for the rows
that must look alike, but Monolith does not promise equal response times,
least of all over Tor. The goal is that the cases cannot be told apart by
what is sent, not by how long it took.

## 13. File transfer

Only between accepted contacts. Nothing is transferred that the receiving
user did not accept.

### 13.1 Exchange

    offerer                               receiver
    FileOffer(id, size, name)  ->
                                          user decides
                               <-         FileAccept(id) or FileReject(id)
    FileChunk(id, data) ...    ->
    FileComplete(id, digest)   ->
                               <-         FileComplete(id, digest)

- `transfer_id` is 16 random bytes chosen by the offerer. An offer whose
  identifier is already in use on the session is a violation.
- `size` is the exact number of bytes that will follow, at most
  `MAX_FILE_SIZE`. Zero is allowed.
- The receiver holds an offer for the user for at most `FILE_OFFER_TIMEOUT`
  and then sends FileReject. Offers above the per-contact or global pending
  limits are answered with FileReject without being shown.
- The user can accept an offer only while fewer than the permitted number
  of transfers are active. Otherwise the offer stays pending and the
  interface says why.
- FileAccept and FileReject must name a pending offer made by the peer.
- FileChunk is valid only from the offerer, only after FileAccept, and only
  while bytes remain. Every chunk must carry exactly
  `min(full chunk, bytes remaining)` bytes, where a full chunk is the size
  section 5.1 gives for the frame parameters, 64490 bytes for the working
  values. Chunks are sequential; there is no offset field.
- The offerer sends FileComplete after the last chunk, with the SHA-256 of
  the whole file. It is valid only when exactly `size` bytes have been
  received.
- The receiver compares the digest with its own. On a match it finishes the
  file (13.3) and sends FileComplete back with the same digest. On a
  mismatch it deletes what it received and sends FileAbort.
- The offerer compares the digest in the returned FileComplete with its
  own. A match means the file was delivered. Anything else, including
  FileAbort, means it was not; the offerer reports that and sends nothing
  further.
- Either side may send FileAbort for a transfer at any time after the
  offer.
- Messages can cross. Each side remembers the identifiers of the last
  `MAX_ENDED_TRANSFER_IDS` transfers that ended on the session, for
  whatever reason. A file message that names one of them is discarded. A
  file message that names an identifier that is neither pending, active
  nor remembered is a violation.
- A session that ends aborts its transfers. There is no resume.

### 13.2 Scheduling and backpressure

The sender writes at most one FileChunk ahead of any waiting chat or control
message, so a transfer delays a chat message by at most one frame. The
receiver stops reading from the session while its disk writer is behind.
There is no application-level window; the Tor stream provides flow control.

### 13.3 Receiver-side handling of the file

- Before accepting: the declared size must not exceed the configured
  maximum, and free disk space must cover it with a margin.
- Data is written to a temporary file whose name Monolith generates, created
  with exclusive-create semantics in a directory only the user can access.
- The received filename is never used as a path. For a save-name suggestion
  the receiver additionally strips or replaces: leading and trailing dots
  and spaces, characters that are reserved on common filesystems
  (`< > : " | ? *`), and reserved device names (CON, NUL, COM1 and so on).
  If nothing usable remains, a generic name is used.
- The final file is created with exclusive-create semantics. An existing
  file is never overwritten; the name gets a numeric suffix instead.
- Nothing is opened, previewed, executed or type-sniffed. No MIME type is
  transmitted, and none would be trusted.

## 14. Duplicate sessions

Both peers may dial each other at the same time. The rule below is applied
only to sessions in `AuthenticatedContact`; an unauthenticated stream cannot
affect any other session.

Identity keys are compared as 32-byte strings, lexicographically.

When a session with contact C becomes `AuthenticatedContact` and another
`AuthenticatedContact` session with C already exists:

- If the same side initiated both: keep the newer one, close the older.
  A side opens a second session only after it considers the first dead, so
  the older one is stale.
- If different sides initiated them: the preferred session is the one
  initiated by the side with the smaller identity key.
  - If the preferred session is the newer one, keep it and close the older.
  - If the preferred session is the older one, it may be dead without this
    side having noticed. Send a Ping on it. If the Pong arrives within
    `DUPLICATE_PROBE_TIMEOUT`, keep it and close the newer one. If not,
    close it and keep the newer one.

The losing session is closed with Close. Messages that were not acknowledged
on it are resent on the surviving session; the receiver drops any it had
already received by identifier. Transfers on the losing session are aborted.

When both sessions are alive, both ends reach the same result, because the
preference depends only on the two identity keys and on who initiated. When
one of them is dead, only the side that still holds it has a decision to
make, and the probe settles it.

To keep duplicates rare, a side does not dial a contact with whom it has an
authenticated session, and cancels a dial in progress when an inbound
session with that contact authenticates.

Before confirmation, an identity with no contact record, or one that is
declined or blocked, may have one `AuthenticatedUnknown` session; a second
one is closed. An identity held as requested or accepted may have one
unconfirmed session per direction, so that a simultaneous dial reaches
confirmation on both and the rule above decides. New authenticated sessions
per identity are rate limited (`CONTACT_SESSION_RATE`).

## 15. Reconnecting

Reconnect timing is local policy, given in `RESOURCE_LIMITS.md` section 7.
The protocol requires only that a peer tolerates the other side
reconnecting, and that neither side depends on the other's schedule.

## 16. Test vectors

### 16.1 Vectors that exist

The fingerprint and the contact card were reproduced by a second
implementation written independently of the Rust code, from the field
lists in this document.

Identity: the key pair of RFC 8032 section 7.1, test 1.

    seed      9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60
    identity  d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a

The same key as it appears in a TLS Certificate entry (section 4.2):

    spki      302a300506032b6570032100
              d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a

Fingerprint of that identity (section 10):

    full      YIRR UZHO JELC AIYD AIKQ AHCH XAGT EYDA AX3D VLUH 6WO3 ZSYJ BYYQ
    compact   YIRR UZHO JELC AIYD AIKQ AHCH

Contact card of that identity (section 11): epoch 1, one endpoint, no
invitation. The endpoint is the Ed25519 public key of the seed that
consists of 32 bytes 0x02.

    endpoint   8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394
    signature  781d3a8ae5359cbbd13055d9521ef15d83c02dda6f5dac448f5d4d963e5f8027
               053d23296fd29c4328b34c56cd1123333d0a6a61ad7a88e49e96fe7da0f43200

    signed bytes (99)
               4d4f4e4f4c4954482d434f4e544143542d434152442d5631 01
               d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
               0000000000000001 01
               8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394
               00

    text form  MONOLITH1:AHLVVGABQKYQVN6VJP7NHSLEA45A5YLS6PNKMIZFV4BBU2HXA5IRU
               AAAAAAAAAAAAEAYCOLXB2UH2F27K2RVIZWDJR7MZS4NRKI3J3RXUJO7MD23R7E3
               HFAAPAOTVCXFGWOLXUJQKXMVEHXRLWB4ALO2N5O2YREPLVGZMPS7QATQKPJDFFX
               5FHCDFCZUYVWNCERTGPIKNJQ226UI4SPJN7T5UD2DEAA

The text form is one string; it is wrapped here for the page, and the
parser ignores the line breaks.

Strict verification (section 10.1): a signature whose R component is the
identity element. Verification by the plain Ed25519 equation accepts it for
this key and the message `message` (7 ASCII bytes); a conforming
implementation rejects it.

    seed       0707070707070707070707070707070707070707070707070707070707070707
    key        ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c
    signature  0100000000000000000000000000000000000000000000000000000000000000
               d4c5323131ff9683cfb3d30197c786c9043f1052d6c2d5293f19d6f36b911104

Key validity (section 10.1), as rules for building the inputs:

- a string that does not decompress is rejected, for example y = 2
  (`02` followed by 31 zero bytes);
- each of the eight points of small order is rejected, the identity
  element among them;
- a valid key plus any point of small order other than the identity
  element is rejected;
- every 32-byte string whose y coordinate is p + k for k from 0 to 18, with
  either sign bit, is rejected, and so is the identity element with the
  sign bit set;
- a key derived from a seed is accepted.

Each holds for identity keys and for onion service keys alike.

### 16.2 Vectors still to be produced

- onion address derivation from a key, with the Tor backend;
- frame encoding of every message type at minimum and maximum size;
- a complete handshake with fixed keys and fixed randomness, if a provider
  built for tests makes that practical (ADR 0002, R5). The signed input
  of section 4.3 is defined by RFC 8446, whose traces are in RFC 8448.

## 17. Open questions

P1. Padding block size. 1024 bytes is a judgment call between overhead and
    how much length information leaks. Needs review together with the
    traffic-analysis limits in the threat model.

P2. Whether a session should also be bound to the onion service key the
    connection was made to. It would tie the session to the endpoint as
    well as the identity. Both sides could compare a value from the TLS
    exporter mixed with that key. It complicates endpoint migration, when
    a responder serves two endpoints for a while. Currently not bound.

P3. Epoch after restoring an old backup. A restored identity may hold an
    epoch lower than one it issued later. Proposed handling: a restore
    advances the epoch by 2^32 before the next card is signed. Needs review.

P4. Whether a declined requester should, after all, receive a distinct
    signal. Currently it cannot tell a decline from a pending request, which
    means its client retries indefinitely at the capped interval.

P5. Whether to drop `Profile.profile_text` from version 1.

P6. Receiver behavior when ChatMessage ordering matters across reconnects.
    The current design preserves the sender's queue order and nothing more.

P7. Closed. Display names had to be in Normalization Form C, which is
    decided with the tables of the Unicode version an implementation was
    built with, so two builds could disagree about a name and end a session
    over it. Normalization is no longer part of protocol validity: section
    9 uses only byte lengths, scalar counts and code point lists written
    out in this document. A front end may normalize for presentation.
