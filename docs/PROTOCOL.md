# Monolith protocol, version 1

Status: the protocol core (framing, messages, text rules, session states,
contact confirmation, duplicate resolution) is implemented in
`monolith-protocol`. The session layer is decided in ADR 0002: Noise XK
with a transport key that the identity certifies in the contact card.
Sections 3, 4, 5, 6.1 to 6.3 and 7.1 specify it, section 11 gives the
contact card with the transport key, and `monolith-session` implements it.
This
document was written first and the implementation follows it; where the
two disagree, the implementation is wrong.

The padding block size in section 5 is a parameter whose production value
is not decided. Open questions are listed in section 17.

Related documents: `CRYPTOGRAPHY.md` (primitives, handshake analysis),
`RESOURCE_LIMITS.md` (every numeric limit), `SECURITY_INVARIANTS.md`.

## 1. Overview

Two peers talk over one Tor stream. The initiator opens the stream to the
responder's Onion Service; nothing in the protocol depends on Tor beyond a
reliable ordered byte stream.

A connection goes through these steps:

1. A Noise XK handshake of three fixed-size messages creates an encrypted
   channel and authenticates the transport key of each side. The
   responder is authenticated first (section 4).
2. The contact cards bind the transport keys to Monolith identities
   (sections 4.4, 6 and 11).
3. Messages are exchanged as encrypted, padded frames (section 5), subject
   to the session state (section 7).

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

There is no preamble and no version field on the wire. The first bytes on
the stream are handshake message 1.

The version is part of the handshake prologue (section 4.1), as the label
`MONOLITH-SESSION-V1`. Both sides hash it into the handshake. A peer that
uses another label fails the first message, in the same way as a peer that
uses another key.

- Nothing is negotiated, so nothing can be downgraded.
- A version mismatch cannot be told from any other handshake failure. An
  initiator never retries with another version on its own.
- A later version of the protocol, or an optional extension of this one,
  gets a new label. There are no feature bits.

## 4. Handshake

The handshake is the Noise protocol `Noise_XK_25519_ChaChaPoly_SHA256`, as
defined by the Noise Protocol Framework, revision 34. The initiator is the
Noise initiator and the responder the Noise responder. Nothing in this
section redefines Noise; it fixes the parameters and says what Monolith
checks.

    XK:
      <- s
      ...
      -> e, es
      <- e, ee
      -> s, se

### 4.1 Parameters

Protocol name, 32 ASCII bytes:

    Noise_XK_25519_ChaChaPoly_SHA256

It is exactly as long as a SHA-256 output, so the initial handshake hash
is the name itself.

- DH: X25519 (RFC 7748). Cipher: ChaCha20-Poly1305 (RFC 8439), with the
  96-bit nonce formed as 32 zero bits followed by the 64-bit counter in
  little-endian order. Hash: SHA-256, with HMAC-SHA256 in the Noise HKDF.
- Static keys: the transport key of each side (section 10.2). The
  transport key of a responder is the one stated in its contact card
  (section 11). It is a long-term key.
- Ephemeral keys: generated for each handshake and discarded with it.
- Pre-message: the responder's transport public key, which the initiator
  takes from the contact card it holds for the identity it dials.
- Prologue, 51 bytes:

      "MONOLITH-SESSION-V1"            19 bytes
      responder identity public key    32 bytes

  The initiator uses the identity key of the contact it dials. The
  responder uses its own. Section 4.6 says why the identity key is there.
- Payloads: empty in messages 1 and 2. The payload of message 3 is the
  initiator's contact card (section 4.4).
- No pre-shared key.

### 4.2 Messages

Every message has a fixed size. Each is read as exactly that many bytes.
There is no length prefix and no variable part.

| Message | Direction | Size | Content |
| --- | --- | --- | --- |
| 1 | initiator -> responder | 48 | ephemeral public key (32), tag of the empty payload (16) |
| 2 | responder -> initiator | 48 | ephemeral public key (32), tag of the empty payload (16) |
| 3 | initiator -> responder | 235 | encrypted transport public key (32 + 16), encrypted contact card (171 + 16) |

The ephemeral public key in messages 1 and 2 is sent in clear and must be
a valid X25519 key (section 10.2). The card in message 3 is the binary
form of section 11.1 without invitation capability, which is 171 bytes in
version 1.

Processing, in the terms of the Noise specification, section 5:

    h = protocol name;  ck = h
    MixHash(prologue)
    MixHash(responder transport public key)

    message 1:  e                MixHash(e)
                es               MixKey(DH(initiator e, responder s))
                payload          EncryptAndHash(empty)

    message 2:  e                MixHash(e)
                ee               MixKey(DH(responder e, initiator e))
                payload          EncryptAndHash(empty)

    message 3:  s                EncryptAndHash(initiator transport public key)
                se               MixKey(DH(initiator s, responder e))
                payload          EncryptAndHash(card)

    Split():    first key        frames from initiator to responder
                second key       frames from responder to initiator

An X25519 result of all zeros ends the handshake.

### 4.3 Order

    Initiator                                   Responder

      message 1                         ---->
                                                checks message 1
                                        <----   message 2
      checks message 2: the responder
        holds the transport key of the
        identity that was dialed
      message 3                         ---->
                                                checks message 3 and the
                                                  card in it
      frames                            <--->   frames

The responder proves that it holds its transport key in message 2. The
initiator sends its transport key and its contact card only after that,
in message 3, encrypted so that only that responder can read them.

After message 3 both sides hold two cipher states, one per direction, and
the handshake hash `h`.

### 4.4 Checks

The initiator:

1. Before it sends anything: the contact it dials has a pinned card; the
   identity of that card is not the initiator's own. It does not dial a
   card that it knows to be superseded (section 11.4).
2. Message 2 is 48 bytes, its ephemeral key is valid (section 10.2), and
   Noise accepts it. A message 2 that Noise accepts shows that the sender
   holds the transport key of the pinned card and used the pinned
   identity key in its prologue.

If check 2 fails the initiator closes. It has sent 48 bytes that carry no
identity. The failure is reported to the user as an identity mismatch: the
service at the contact's address did not prove the contact's identity. An
initiator cannot tell an impostor from a contact whose transport key has
changed (section 11.4) or from a service that is not Monolith at all, and
the interface says so. This is never resolved automatically, and there is
no option to continue.

The responder:

1. Message 1 is 48 bytes, its ephemeral key is valid, and Noise accepts
   it. A message 1 that Noise accepts shows that the sender knows the
   responder's transport public key and identity public key. Both are
   public data; see section 4.6.
2. Message 3 is 235 bytes and Noise accepts it. Noise then knows the
   initiator's transport public key and has verified that the initiator
   holds the matching private key.
3. The payload is a valid contact card (section 11.2) that carries no
   invitation capability.
4. The transport key in the card is byte for byte the transport key Noise
   authenticated in step 2.
5. The identity key in the card is not the responder's own.

When these hold, the initiator's identity is the identity key of the
card. The responder checks nothing else here. In particular it does not
look at its contacts, its block list or any other record of the identity,
so the handshake behaves the same for every initiator with a valid card.

What the responder holds about the identity is looked at afterwards, to
decide the standing of the peer for this session (section 6.2). That
includes the epoch of the card that was presented.

### 4.5 Bounds and failures

- A side reads exactly the bytes of the message it expects: 48 for
  message 1, 48 for message 2, 235 for message 3. It reads nothing more
  before the handshake is complete.
- The handshake must complete within `HANDSHAKE_TIMEOUT`.
- Any failure ends the handshake by closing the stream. Nothing is sent:
  Noise has no error messages and Monolith adds none. Every failure looks
  the same to the peer, whichever check failed.
- The cost a caller can impose: with a first message that Noise rejects,
  one X25519 operation. With a valid first message, two X25519 operations
  and one key generation, and one pending handshake until message 3
  arrives or the timeout expires. With a valid third message, one more
  X25519 operation and the validation of one contact card.

### 4.6 What the keys in the handshake stand for

Noise authenticates transport keys. Two things bind them to identities.

The contact card. An identity states its transport key in its card and
signs the card. The initiator holds the responder's card before it dials.
The responder receives the initiator's card in message 3 and checks that
its transport key is the one Noise authenticated.

The prologue. A card says that an identity vouches for a transport key. It
does not say that the holder of the transport key agrees to be that
identity. Without something more, an identity could sign a card that names
another party's endpoint and transport key. A peer that imported that card
would reach the other party, complete the handshake, and attribute the
session to the wrong identity. The responder therefore puts its own
identity key into the prologue, and the initiator the identity key of the
card it dialed. If they differ, message 1 fails. For the initiator's side
the same binding comes from message 3: the card is inside the handshake,
so it is the holder of the transport key who presents it.

Knowing a responder's transport key and identity key is not possession of
a secret. Both are in every contact card. A caller without them gets no
reply at all, which keeps a party that has only the onion address from
learning anything, and that is all it does. It is not access control.

## 5. Frames

After the handshake every transmission is a frame:

    u16 length || ciphertext[length]

`length` must satisfy

    1040 <= length <= 64528  and  (length - 16) mod 1024 == 0

and, while the session is in `AuthenticatedUnknown`, `length` must be
exactly 1040. No frame is accepted before that state. A length that fails
these checks is a violation and is detected before the ciphertext is read.

The ciphertext is one Noise transport message: the plaintext encrypted with
the cipher state of that direction (section 4.2), nonces counting from
zero, empty associated data. A decryption failure is a violation. Frames
cannot be reordered, dropped or replayed without causing one, and a frame
of another session does not decrypt.

The plaintext is:

    u16 type || u16 body_length || body[body_length] || padding

- `padding` is zero bytes up to the next multiple of 1024. The plaintext
  length must equal `4 + body_length` rounded up to a multiple of 1024; more
  padding than necessary, or a non-zero padding byte, is a violation.
- `type` must be an assigned code (section 8). An unassigned code is a
  violation. There are no ignorable message types in version 1; a type that
  is added later needs a new protocol label (section 3).
- The message must be legal in the current session state (section 7). This
  is checked before the body is parsed.

One frame carries one message. No message spans frames.

### 5.1 Frame parameters

Two numbers in this section are parameters, not constants of the design:

- P, the padding block. This document uses P = 1024. That value is
  provisional; ADR 0003 gives the trade-off and the alternatives.
- T, the number of bytes the session layer adds to each frame: 16, the
  authentication tag of a Noise transport message.

In terms of them:

- the plaintext length is a multiple of P, at least P, and at most the
  largest multiple of P that keeps `length` within 65535;
- `length` is the plaintext length plus T;
- before a session is confirmed, the plaintext is no longer than the padded
  size of the largest message that is legal then. That message is a
  ContactRequest of 832 bytes, 836 with its header. For P = 1024 this is
  one block, which is where "exactly 1040" comes from.

A pair of values is usable only if P is larger than the 4-byte message
header, P + T is at most 65535, and the largest plaintext can hold every
message other than a FileChunk at its largest size. The largest of those is
a ChatMessage of 16402 bytes. An implementation refuses any other pair.

A FileChunk is cut to the frame: a full chunk carries as much data as the
largest body holds after the 16-byte transfer identifier and the 2-byte
length, and never more than 64490 bytes.

The limits 1040, 64528, 64512, 64508 and 64490 elsewhere in this document
are these rules evaluated for P = 1024 and T = 16. An implementation takes
P and T as parameters, so that changing either is a change of two numbers
and not of the frame decoder.

Padding hides the exact length of short messages from anyone who can see
ciphertext lengths on the path between the application and Tor. It is not a
defense against traffic analysis; see `THREAT_MODEL.md`.

## 6. Identity authentication

### 6.1 Authentication by the handshake

The identity of each side is established by the handshake of section 4
and the contact cards. There is no separate identity proof and no message
for one. Message code 0x0001, which earlier drafts gave to such a message,
is not assigned. No signature is made or verified during a session other
than the signature of a contact card.

- For the initiator, the peer's identity is the identity of the card it
  dialed. Message 2 proved it.
- For the responder, the peer's identity is the identity key of the card
  in message 3, after the checks of section 4.4.

### 6.2 Result and standing

When the handshake and the checks of section 4.4 have succeeded, both
sides are in `AuthenticatedUnknown`: the peer's identity is proven, and
whether the two are contacts has not yet been confirmed on this session.
Section 6.4 says how a session leaves that state.

An initiator has no confirmation that the responder accepted message 3
until the first frame from the responder decrypts. A responder can
produce such a frame only if it processed message 3.

Authentication says who the peer is. It does not say what the peer may
do. A peer with a valid card that the local side has never seen, has
declined, has blocked or has deleted is authenticated exactly like a
contact, and is then handled by sections 6.4 and 12.

Each side decides the standing of the peer from its own record of the
identity and from the card that stands for the peer on this session. For
a responder that is the card presented in message 3. For an initiator it
is the card it dialed.

For an identity held as a contact, the record includes the newest card the
local side holds of it: the pinned card, or a card with a greater epoch
that arrived later and that the user has not confirmed yet (section 11.4).
The comparison is with the newest one, confirmed or not.

| Record of the identity | Card of this session | Standing for this session |
| --- | --- | --- |
| none, declined or blocked | any valid card | not a contact (section 12) |
| requested or accepted | greater epoch than the newest card held | as the record says; the card becomes the newest card held at once, and is a pending change (section 11.4) |
| requested or accepted | same epoch, same transport key and endpoints | as the record says |
| requested or accepted | same epoch, another transport key or endpoint set | not a contact; reported to the user as a conflict |
| requested or accepted | lower epoch than the newest card held | not a contact |

The last two rows are the stale-card rule. A card that is older than the
newest one the local side holds, or that contradicts it, does not open a
contact session, even though its signature is valid and the peer holds its
transport key. The peer is treated like any identity that is not a contact
and sees the same generic behavior (section 12.1); it is not told why.
This keeps a transport key that an identity has retired from being used
against the contacts that hold its successor, whether or not their users
have confirmed the successor yet.

For an initiator the rule matters when a newer card arrived while a dial
was in progress: the responder then proved a transport key that is known
to be retired, and the session is not a contact session.

Budgets are applied at this point according to the standing for this
session, not according to the record alone. A session whose standing is
requested or accepted counts against `MAX_CONTACT_SESSIONS`. Every other
session, that of a contact with a stale card included, counts against
`MAX_UNKNOWN_SESSIONS` and `UNKNOWN_SESSION_RATE`; if the rate is
exhausted, the local side closes. Holders of a contact card who are not
contacts therefore cannot use up the room that contacts need, and the
holder of a retired key cannot use up the room or the rate of the identity
it belongs to. They can use up the room for other strangers; see
`RESOURCE_LIMITS.md` section 12.

No other check is applied here. In particular a blocked identity is not
turned away at this point; it is handled like any other identity that is
not a contact (section 12), so that being blocked cannot be told from being
unknown.

### 6.3 Extensions

Version 1 has no optional extensions and no negotiation. An extension that
is added later is a new protocol label (section 3). Labels describe
protocol behavior only and are never used to convey platform, build or
product information.

### 6.4 Contact confirmation

What a side sends first in `AuthenticatedUnknown` depends only on what it
holds locally about the peer's identity, through the standing of section
6.2:

| Standing of the peer | First message |
| --- | --- |
| accepted contact | ContactAccept |
| requested (the user imported the peer's card; no acceptance seen yet) | ContactRequest |
| none, declined, blocked, or a contact whose card is stale for this session | nothing |

A session becomes `AuthenticatedContact` on a side when both of these are
true: the side holds the peer as an accepted contact, and it has received
ContactAccept from the peer on this session. A side sends no message other
than ContactAccept, ContactRequest and Close before that.

A side that was confirmed before it sent its own ContactAccept, because it
held the peer as requested and the peer's ContactAccept arrived, sends its
ContactAccept next. Until that has gone out it sends nothing but Close:
the peer takes contact-only messages only after it has received the
ContactAccept of this side.

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
- Anything from a peer with no record, declined or blocked, or from a
  contact whose card is stale for this session: section 12.

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
| CryptoHandshake | Noise handshake messages are exchanged | none (the three fixed-size handshake messages only) |
| IdentityAuth | Noise handshake complete and the card checks of section 4.4 passed; the identity is being bound to the session and the standing of section 6.2 applied | none |
| AuthenticatedUnknown | Peer's identity proven; contact relationship not confirmed on this session | ContactRequest, ContactAccept, Close |
| AuthenticatedContact | Both sides hold each other as accepted contacts and have said so on this session | every message type |
| Closing | Close sent; the stream is being shut down | none |
| Closed | Terminal | none |

A session never moves backwards, and every session passes through
`AuthenticatedUnknown`. The implementation of this table is
`MessageType::may_be_received_in` and `SessionState::can_transition_to` in
`monolith-protocol`; the logic of sections 6.4 and 12 is `session::Session`.

A session enters `AuthenticatedUnknown` only for the identity that the
handshake established (section 6.1), and only through a completed
handshake. The session ends if that identity is the local one, and, on a
session the local side opened, if it is any identity other than the one
that was dialed. An implementation does not let a caller move a session
into `AuthenticatedUnknown` by any other route.

A Close that is received ends the session at once: the receiver goes to
`Closed`. After a Close was sent or received, a side stops reading from the
stream. Bytes still in flight are dropped without being decoded.

A session leaves `AuthenticatedUnknown` within `UNKNOWN_SESSION_TIMEOUT`,
by confirmation or by closing.

`Session` in `monolith-protocol` is the logic of this table and holds no
key. The object that exists only for a peer that completed a handshake,
and the only one that can read or write a frame, is `AuthenticatedSession`
in `monolith-session`. That is what the rest of an implementation holds.

### 7.1 Session limits

A session ends when the first of these is reached, and a new handshake
replaces it:

| Limit | Value |
| --- | --- |
| Age | 24 hours (`MAX_SESSION_LIFETIME`); 48 hours while a file transfer is active (`MAX_SESSION_LIFETIME_WITH_TRANSFER`) |
| Frames one side sends | 2^32 (`MAX_FRAMES_PER_DIRECTION`) |
| Ciphertext bytes one side sends | 2^40 (`MAX_BYTES_PER_DIRECTION`): the sum of the frame length fields |

Both sides apply the same values. They are constants of the protocol and
are not negotiated.

- A side that has reached a limit sends nothing but Close. Of the frame
  and byte limits it keeps one frame and 1040 bytes back, so that the
  Close always fits.
- Each side counts the age from the moment it completed the handshake, on
  its own clock. The two moments are up to one handshake apart.
- The longer age limit applies while a file transfer is active, and only
  if the transfer was already active when the session reached 24 hours. A
  transfer that would begin later does not extend the session, and no
  transfer is started on a session that is past 24 hours. When the
  transfer ends, the ordinary limit applies again, so a session past 24
  hours then sends nothing but Close.
- A receiver takes frames until `SESSION_CLOSE_GRACE` after the age limit
  that applies. This lets the last frames that were sent in time, and the
  Close, arrive from a peer whose clock started a little later. A frame
  that arrives after that is a violation. So is a frame beyond the frame
  or byte limit.
- There is no rekey and no way to reset a counter. There is no session
  resumption.

## 8. Messages

All sizes are body sizes. "States" lists where the message may be received.

| Code | Name | Body size | States |
| --- | --- | --- | --- |
| 0x0002 | Close | 0 | Unknown, Contact |
| 0x0003 | Ping | 8 | Contact |
| 0x0004 | Pong | 8 | Contact |
| 0x0010 | ContactRequest | 176 to 832 | Unknown, Contact |
| 0x0011 | ContactAccept | 0 | Unknown, Contact |
| 0x0020 | ChatMessage | 19 to 16402 | Contact |
| 0x0021 | MessageAck | 16 | Contact |
| 0x0030 | Profile | 4 to 1156 | Contact |
| 0x0031 | EndpointUpdate | 171 | Contact |
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

    [171]        card           the sender's contact card, no capability
    u8           has_capability 0x00 or 0x01
    [16]         capability     present only if has_capability is 0x01
    text<0..128> display_name
    text<0..512> introduction

`card` must be a valid contact card (section 11) whose identity key and
transport key are those the sender authenticated in the handshake of this
session, and it must not carry a capability of its own. If the sender is
the initiator, the card is byte for byte the card of message 3. A request
whose card fails these checks is a protocol violation: the stream is
closed and nothing is sent. The receiver makes this check before it looks
at what it holds about the sender, so the outcome is the same for a
stranger, a blocked identity and a contact.

`capability` is the invitation capability copied from the card of the peer
being asked (section 12): on a session the local side opened, the card
that was dialed; on one the peer opened, the card of the peer that the
local side holds. A request with any other capability is not sent. Which
card that is when the user holds several cards of one identity is open
question P10.

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

    [171] card   the sender's contact card, no capability

`card` must be a valid contact card whose identity key equals the sender's
proven identity; a card of another identity is a protocol violation. The
card may state another transport key than the one this session was
authenticated with: that is how a new transport key is announced. The
receiver compares the card with the newest card it holds of this contact:

- greater epoch: recorded as a pending change of the endpoint set, of the
  transport key, or of both. In version 1 the change takes effect for
  dialing after the user confirms it. For the stale-card rule it counts
  from the moment it is recorded (section 11.4);
- same epoch, same endpoint set and same transport key: nothing to do.
  This is the normal case;
- same epoch and a different endpoint set or transport key: the owner
  signed two statements with one epoch. Ignored, and reported to the user
  as an anomaly;
- lower epoch: ignored and counted.

Nothing but a greater epoch ever changes what is pinned or what is held as
the newest card.

A responder sends its current card once after a session is confirmed. The
initiator's card was presented in the handshake. Either side sends its
card again if it changes during the session. This is how a contact learns
a new endpoint or a new transport key: the owner dials out and says so.

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

### 10.2 Valid X25519 keys

A 32-byte string is a valid X25519 public key for Monolith only when both
of these hold:

1. It is the canonical encoding of a field element: read as a
   little-endian integer it is less than 2^255 - 19. The top bit of the
   last byte is therefore zero.
2. It is not one of the five points of small order:

       0000000000000000000000000000000000000000000000000000000000000000
       0100000000000000000000000000000000000000000000000000000000000000
       e0eb7a7c3b41b8ae1656e3faf19fc46ada098deb9c32b1fd866205165f49b800
       5f9c95bca3508c24b1d0b1559c83ef5b04445cc4581c8e86d8224eddd09f1157
       ecffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff7f

The rule applies to the transport key in a contact card, to the transport
key an initiator sends in message 3, and to the ephemeral keys in messages
1 and 2.

RFC 7748 accepts any 32 bytes as a public key: it ignores the top bit and
reduces the rest. Monolith requires the canonical form so that a key, and
with it a contact card, has exactly one encoding. With a point of small
order the shared secret is zero whatever the private key is, so such a
point proves nothing about who holds it. Whether a point lies on the curve
or on its twist is not checked; X25519 is safe for both, and RFC 7748 asks
for no such check.

A transport key is generated from 32 bytes of CSPRNG output of its own. It
is not derived from the identity key, and it is used for nothing but the
handshake of section 4.

## 11. Contact card

A contact card is a signed statement by an identity: "as of this epoch, my
sessions are authenticated by this transport key, and I can be reached at
this set of endpoints".

There is one card format. A card is the same bytes however the user
distributes it: handed to one person, shown as a QR code, put on a website
or listed in a directory. The channel is the user's decision and is not
part of the protocol. An identity may issue several cards at the same
time that differ only in their invitation capability (section 12.2).

### 11.1 Binary form

With n = `endpoint_count`:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 1 | version = 0x01 |
| 1 | 32 | identity_public_key (Ed25519) |
| 33 | 32 | transport_public_key (X25519) |
| 65 | 8 | epoch (`u64`, at least 1) |
| 73 | 1 | endpoint_count (n) |
| 74 | 32 x n | endpoints: one onion_service_key each |
| 74 + 32n | 1 | flags |
| 75 + 32n | 0 or 16 | invitation_capability |
| then | 64 | signature |

`endpoint_count` is at least 1 and at most `MAX_ACTIVE_ENDPOINTS`. In
version 1 that maximum is 1, so every valid card has exactly one endpoint
and is 171 bytes long without a capability, 187 with one. The endpoints in
a card are distinct.

`transport_public_key` is the X25519 key with which the identity
authenticates its sessions (section 4).

`flags`: bit 0 set means `invitation_capability` is present. Bits 1 to 7 are
zero; a card with any of them set is invalid.

An `onion_service_key` is the 32-byte Ed25519 public key of an Onion
Service v3. The address is derived from it as the Tor specification
defines:

    checksum = SHA3-256(".onion checksum" || onion_service_key || 0x03)[0..2]
    address  = base32(onion_service_key || checksum || 0x03) || ".onion"

Storing the key instead of the address means a card cannot carry a wrong
checksum or version byte. Only version 3 services can be expressed.

The version byte is 0x01. Drafts of this document before the session layer
was decided described a card without the transport key, 139 or 155 bytes
long. No release produced such cards, and they are not valid.

### 11.1.1 Signed bytes

`signature` is an Ed25519 signature by `identity_public_key` over the
signed bytes of the card. The signed bytes are defined here, field by
field, and not by reference to the transport layout:

    "MONOLITH-CONTACT-CARD-V1"   24 bytes
    version                       1 byte
    identity_public_key          32 bytes
    transport_public_key         32 bytes
    epoch                         8 bytes, big-endian
    endpoint_count                1 byte
    endpoints                    32 bytes each, in card order
    flags                         1 byte
    invitation_capability         0 or 16 bytes

In version 1 these are the bytes of the binary form up to the signature,
after the prefix. An implementation still builds them with a function of
its own, separate from the transport encoder, so that a later change to
the transport layout cannot change what a signature means. With one
endpoint the signed bytes are 131 or 147 bytes long.

This is the only signature the identity key makes.

### 11.2 Validation

In this order; the first failure rejects the card:

1. Length is at least the size of the fixed fields.
2. `version` is 0x01.
3. `epoch` is at least 1.
4. `endpoint_count` is between 1 and `MAX_ACTIVE_ENDPOINTS`.
5. `flags` has no reserved bit set.
6. The length is exactly what `endpoint_count` and `flags` imply. There are
   no trailing bytes.
7. `identity_public_key` is a valid key (section 10.1).
8. `transport_public_key` is a valid X25519 key (section 10.2), and it is
   not the Montgomery form of `identity_public_key` or of an endpoint.
9. Every endpoint is a valid onion service key (section 10.1), no two are
   equal, and none is byte for byte equal to `identity_public_key`.
10. The signature is valid (section 10.1) over the signed bytes built from
    the decoded fields.

The decoder accepts exactly one encoding of a card, so the signed bytes
built from the decoded fields are determined by the received bytes and by
nothing else.

Key separation. A Monolith identity has three keys with three jobs:

    identity key    Ed25519   signs contact cards and nothing else
    transport key   X25519    authenticates sessions (section 4)
    onion key       Ed25519   reachability; used by Tor under Tor's rules

They belong to different cryptographic domains and are generated
independently. A key must never be used in two of them. A card states the
three public keys side by side, so it can show a violation: an endpoint
that is the identity key, or a transport key that is the identity key or
an endpoint key carried over to the other curve form. The Montgomery form
of an Ed25519 public key with coordinate y is u = (1 + y) / (1 - y) modulo
2^255 - 19, encoded in 32 bytes, little-endian. A card with such a
coincidence is invalid, and an implementation refuses to sign one. This is
invariant S33 in `SECURITY_INVARIANTS.md`.

The rule catches a key that is reused on purpose or by a bug in key
handling. It cannot catch keys derived from one secret by other means;
`CRYPTOGRAPHY.md` section 3 requires the keys to be generated
independently.

### 11.3 Text form

    "MONOLITH1:" || base32(card)

Base32 uses the RFC 4648 alphabet without padding. The canonical form is
upper case, which lets a QR code use alphanumeric mode. A card of version
1 is 284 characters long, 310 with a capability.

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
from anyone who holds the card. Cards and QR codes are produced locally,
and Monolith never uploads, publishes or registers a card anywhere
itself. The user may hand a card to one person or publish it; a published
card is known, with its endpoints, its keys and its capability, to
everyone who can obtain it (section 12.2).

### 11.4 Epochs, endpoint sets and transport keys

A card states the whole of what an identity publishes at one moment: one
transport key and one set of endpoints. The epoch orders these statements.

An identity has a set of endpoints, not one endpoint. The model is

    identity -> transport key and endpoint set, as of an epoch

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

The owner of an identity keeps a counter. Each time its endpoint set or
its transport key changes it increments the counter and signs a new card.
A card always states the whole set and the transport key; there is no
"add" or "remove".

A receiver pins, per contact, the identity key, the transport key, the
endpoint set and the epoch: the card that is in effect, which is the one
it dials. Next to it, it holds the newest card of that identity it has
received. The two are the same card except while a change is pending.

A card signed by the pinned identity with an epoch strictly greater than
that of the newest card held becomes the newest card held at once, when it
arrives

- as the card an initiator presents in the handshake of a session with
  that contact,
- in an EndpointUpdate on an authenticated session with that contact, or
- as a card the user imports by hand.

In version 1 it becomes the pinned card when the user confirms the change;
a card the user imports by hand is confirmed by that. Until then the
contact is not dialed: the pinned card is known to be superseded, and the
new one is not in effect. What the user confirms is where Monolith
connects to. What counts as stale does not wait for the user.

A card with a different identity key is a different contact, whatever
display name comes with it.

Two cards of one identity with the same epoch must be identical in
transport key and endpoint set. If they are not, the identity has signed
two statements for one epoch; the receiver keeps what it has and reports
the conflict. They may differ in their invitation capability, and then in
their signature: an identity may issue several cards for one epoch, one
per capability (section 12.2). Such cards are not a conflict. The
capability is not part of what a receiver pins.

Stale cards. A card whose epoch is lower than that of the newest card held
is never accepted as the current statement of a contact. Presented in a
handshake, it does not open a contact session (section 6.2). Received in
an EndpointUpdate, it is ignored (section 8.8). A receiver that has no
record of the identity cannot know that a card is stale: it has nothing to
compare it with.

Changing the transport key. An identity replaces its transport key by
signing a card with a greater epoch that states the new key. From that
moment it answers only handshakes made with the new key. Version 1 has no
period in which both keys are answered.

- A contact that still holds the previous card cannot open a session: its
  first message is made for a key the identity no longer uses, and it
  gets no reply. To the contact this looks like any other identity
  mismatch (section 4.4).
- The contact learns the new card when the identity opens a session to it
  and presents the card in the handshake, or out of band. Until then the
  two can talk only when the identity dials.
- There is no revocation. Replacing a transport key does not make the
  old one worthless to someone who obtained its private half: against a
  party that has received the newer card it is useless, by the stale-card
  rule, and against a party that has never seen the identity, or holds
  only the older card, it still authenticates as the identity.
  The same is true of the identity key itself, which cannot be replaced
  at all without becoming a new identity.

If every endpoint a contact knows has disappeared before an update reached
it, there is no way for the contact to learn the new endpoint from the
network. The user has to hand over a new card out of band.

## 12. Contact requests and invitations

An invitation capability is 16 random bytes that the user's card may carry.
It is a bearer capability: it allows whoever holds it to attempt a contact
request to the owner of the card, and nothing else. It is an anti-spam
token. It is not an identity, it authenticates nobody, and it does not
replace the handshake: who a peer is comes from section 4 alone.
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

The capabilities a user currently accepts form the active set, of at most
`MAX_ACTIVE_INVITATIONS` members. A capability is valid while it is in
that set. Section 12.2 says what a capability means and how the cards that
carry one are used, section 12.3 what revoking one does.

Processing of the first message received in `AuthenticatedUnknown` from a
peer that is not a contact for this session: one the local side neither
requested nor accepted, or a contact whose card is stale (section 6.2).
Peers whose standing is requested or accepted are handled by section 6.4.

1. Validate the body, including that the card in a ContactRequest is the
   sender's own (section 8.3). A malformed message is a violation.
2. Send Close. This happens whether a request will be queued or dropped and
   whatever the reason, so the sender cannot tell the cases apart. A
   blocked sender sees exactly what a stranger with a bad invitation sees.
3. If the message is a ContactRequest, decide whether to queue it. The
   request is dropped if the sender is on the block list or the declined
   list, if the sender is a contact whose card is stale, if the policy
   mode does not admit it, if it carries a capability that is not in the
   active set (compared in constant time with every member), if a request
   from this identity is already pending, if
   `MAX_PENDING_REQUESTS_PER_INVITATION`
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

The quota per capability keeps one capability, leaked or published, from
filling the whole queue: requests that carry it occupy at most
`MAX_PENDING_REQUESTS_PER_INVITATION` entries at a time, however many
identities hold it.

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
| a contact that presented a stale or conflicting card (section 6.2) | nothing | Close | Close | Close at `UNKNOWN_FIRST_MESSAGE_TIMEOUT` | never |
| requested by the local user | ContactRequest | ContactAccept | ContactAccept | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |
| accepted, verified out of band | ContactAccept | nothing more | confirmed | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |
| accepted, not verified | ContactAccept | nothing more | confirmed | Close at `UNKNOWN_SESSION_TIMEOUT` | after confirmation |

For every row: the handshake is the same, a protocol violation ends the
stream with nothing sent, and Close has an empty body.

Requirements that follow:

- The first five rows are the same row. Message types, their number and
  order, and the conditions under which the session is closed must not
  depend on which of the five applies. The implementation takes one code
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
- When the budget for strangers is exhausted, the first five rows are closed
  right after authentication while the last three are not. This separates
  the same two groups that the messages already separate.

Timing is not part of this guarantee. The same steps are taken for the rows
that must look alike, but Monolith does not promise equal response times,
least of all over Tor. The goal is that the cases cannot be told apart by
what is sent, not by how long it took.

### 12.2 Invitation capabilities and cards

Meaning. Holding a capability means that the holder may attempt a contact
request to the owner of the card, and that in invitation mode the request
is considered. It does not mean that the holder is a particular identity.
A request comes from whatever identity completed the handshake (section
4), and the user still accepts, declines or blocks it. A capability is an
authorization to knock, not a proof of identity.

Monolith makes a capability from `INVITATION_CAPABILITY_LEN` (16) bytes of
operating system CSPRNG output. Its size and its place in the card
(section 11.1) are fixed; nothing in this section changes the wire format.

Distribution. Where a card goes is the user's decision: to one person, as
a QR code, on a website, in a public directory, or over any other channel.
The protocol does not assume that a capability stays confidential. A
capability in a card that was published is known to everyone who can
obtain that card, and no document or interface calls it secret after that.
What it still provides is that only holders of that card can send a
request that passes the invitation check. Knowing the onion address is not
enough; without the card a caller does not even get a handshake reply
(section 4).

Reuse. A card is not a one-time invitation. A capability is valid for any
number of requests from any number of identities until it is revoked; it
is not consumed by use. Version 1 has no use counter, no single-use
capability and no expiry time. Any of them would need a later protocol
version. What bounds the use of one capability is the quota per
capability, the budgets and the rates (`RESOURCE_LIMITS.md` sections 5 and
6).

Several cards for one identity. An identity may issue several cards at the
same time that differ only in their capability, and therefore in their
signature. They state the same identity key, transport key, endpoint set
and epoch, and each is a valid card on its own:

    identity
      +-- card A   capability A   published in directory A
      +-- card B   capability B   published somewhere else
      +-- card C   capability C   handed to one person

All of them identify the same identity, and the owner's active set holds
A, B and C. A receiver treats them as one statement of that identity
(section 11.4): they are not a conflict, and the capability is not part of
what it pins.

Directory cards. A user may make a card for one directory or one website
only, so that it can be withdrawn on its own later. Withdrawing it, by
revoking its capability, changes nothing about the identity, the transport
key, the Onion Service or the accepted contacts. Monolith neither requires
nor trusts a directory. A directory is an optional means of discovery
outside the protocol, and Monolith has no directory client.

Removing a listing is not revocation. A card taken down from a directory
or a website is still held by everyone who copied it, with its capability.
Only revoking the capability locally (section 12.3) stops new requests
that carry it. No document or interface may suggest that removing a
listing withdraws a card. Neither of the two stops a holder from
connecting to the endpoint and seeing whether it is reachable (section
11.3); only a change of the endpoint does that.

Open cards. A card without a capability is an open card: holding it is
enough to attempt a request. In open mode such a request is considered,
subject to the budgets, the rates, the quota of requests without a
capability, and acceptance by the user. In invitation mode it is dropped.
Whether a card carries a capability is the user's choice. Monolith never
adds one to a card the user issued without one, and never removes one.

The mode applies to the identity, not to a card, because a request does
not say which card it was taken from. In open mode the capabilities of
other cards therefore keep nobody out. A client that follows section 8.3
sends the capability of the card it holds, and a request with a revoked
one is dropped; but nothing forces a client to send one, and a request
without a capability is considered in open mode. A revocation is a
barrier only in invitation mode.

Epochs. A capability is not tied to an epoch. A ContactRequest carries the
capability but not the card it was taken from, so the receiver cannot tell
which card or which epoch it came from. When the identity changes its
transport key or its endpoint set, every card it has handed out or
published states the old ones, and a holder with nothing newer cannot
reach it with that card (section 11.4). New cards for the new epoch may
carry the capabilities of the old ones.

Labels. The user may give each capability a local label, such as
"Website", "Directory A" or "Private QR for Bob", to manage the set. A
label is local data. It is not part of the card, is not signed and is
never sent. Putting it into the card or onto the wire would need a
reviewed requirement and a new card version.

Handling. A capability remains a sensitive value inside Monolith even when
the user has published it (`CRYPTOGRAPHY.md` section 8, S18). Publishing
is a decision about where one card goes. It changes nothing in how
Monolith holds, compares, logs or erases capabilities.

### 12.3 The active set and revocation

The active set holds the capabilities the user currently accepts, at most
`MAX_ACTIVE_INVITATIONS`. A capability enters it when the user creates a
card with a new capability, and leaves it only when the user revokes it.
When the set is full, creating another capability fails until the user
revokes one; Monolith never revokes a capability by itself to make room.
The maximum is provisional (`RESOURCE_LIMITS.md` section 5).

Revoking a capability removes it from the active set. From then on a
request that carries it is dropped like one whose capability was never
issued. A request is checked against the set as it is when the request is
decided, which is after the Close (section 12, step 3).

A signature is not an authorization. A card whose capability was revoked
is still a validly signed card. It proves that the identity issued it,
and it still states the identity's keys and endpoints, so its holders can
still connect and complete a handshake. It does not show that its
capability is accepted today. Only the owner's active set says that, and
the owner tells nobody.

Revocation decides about new requests and about nothing else. It does not

- delete or change an accepted contact, or a peer the user requested;
- end a session, or change how a session proceeds;
- revoke or change the identity key, the transport key or the Onion
  Service;
- remove message history.

A contact's authorization comes from the contact relationship and the
identity proven in the handshake (section 6.4), not from the capability
it used to ask. Sessions with contacts and with peers the user requested
never look at a capability.

Requests that are already in the queue were decided before the
revocation, and they stay until the user accepts, declines or blocks
them. Whether revoking should also offer to discard them is open question
P11.

No revocation oracle. A peer gets the same answer, Close, whether its
request carried no capability, one that was never issued, one that was
revoked, one taken from the card of another identity, or one of another
card of this identity, and whether its request was queued or dropped
(section 12.1). No message, field or code says "revoked", "expired" or
"unknown invitation", and none may be added for a peer that is not a
confirmed contact. A capability field that is not well formed (a presence
byte other than 0 or 1, or a body of the wrong length) makes the message
malformed, and the stream is closed with nothing sent, as for every
violation. That depends only on bytes the sender chose and tells it
nothing about the active set.

Inside Monolith the reason a request was dropped may be kept as a typed
value, for counters and for the user's own view. It carries no capability
bytes and no data of the peer, is never sent, and is never logged with
either (S18). Telling "revoked" from "never issued" would need a record of
revoked capabilities. Nothing in the protocol needs one, and version 1
keeps none.

Local state. Revocation needs the active set and nothing else: each member
with its local label, stored with the identity (in the vault, or in memory
for an ephemeral identity). A revoked capability is removed, not marked.

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

These slots and this rate are those of sessions whose standing is
requested or accepted. A session of a contact that presented a stale or
conflicting card (section 6.2) is counted with the sessions of identities
that have no record. It takes no slot and no rate from the identity's
contact sessions, so the holder of a retired key cannot keep the identity
itself out.

## 15. Reconnecting

Reconnect timing is local policy, given in `RESOURCE_LIMITS.md` section 7.
The protocol requires only that a peer tolerates the other side
reconnecting, and that neither side depends on the other's schedule.

## 16. Test vectors

### 16.1 Vectors

Every value below was produced by an implementation written independently
of the Rust code, from this document, RFC 7748, RFC 8032, RFC 8439 and the
Noise specification. The handshake was also reproduced with a Noise
library. All keys are test values and must never be used.

Two parties. R is the responder and I the initiator.

    R identity seed     9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60
    R identity          d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
    R transport secret  77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a
    R transport         8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a
    R endpoint          8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394

    I identity seed     4ccd089b28ff96da9db6c346ec114e0f5b8a319f35aba624da8cf6ed4fb8a6fb
    I identity          3d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af4660c
    I transport secret  5dab087e624a8a4b79e17f8b83800ee66f3bb1292618b6fd1c2f8b27ff88e0eb
    I transport         de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f
    I endpoint          ed4928c628d1c2c6eae90338905995612959273a5c63f93636c14614ac8737d1

The identity keys are tests 1 and 2 of RFC 8032 section 7.1. The transport
keys are the two key pairs of RFC 7748 section 6.1. The endpoints are the
Ed25519 public keys of the seeds that consist of 32 bytes 0x02 and 0x03.

Fingerprint of R's identity (section 10):

    full      YIRR UZHO JELC AIYD AIKQ AHCH XAGT EYDA AX3D VLUH 6WO3 ZSYJ BYYQ
    compact   YIRR UZHO JELC AIYD AIKQ AHCH

Montgomery form of R's identity key (section 11.2), which a card of R must
not state as its transport key:

    d85e07ec22b0ad881537c2f44d662d1a143cf830c57aca4305d85c7a90f6b62e

Contact card of R (section 11): epoch 1, one endpoint, no invitation.

    signed bytes (131)
               4d4f4e4f4c4954482d434f4e544143542d434152442d5631 01
               d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a
               8520f0098930a754748b7ddcb43ef75a0dbf3a0d26381af4eba4a98eaa9b4e6a
               0000000000000001 01
               8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394
               00

    signature  de2f9f4d97b9083972d602de79f255a8824e9400be6e57b841f02f1bd9b3c8fa
               13b89ba8aae47e229e3e49df1a422f57702d740f578452f1d7643c7321c94c03

    text form  MONOLITH1:AHLVVGABQKYQVN6VJP7NHSLEA45A5YLS6PNKMIZFV4BBU2HXA5IRV
               BJA6AEYSMFHKR2IW7O4WQ7POWQNX45A2JRYDL2OXJFJR2VJWTTKAAAAAAAAAAAA
               CAMBHF3Q5KD5C5PVNI2UM3BUY7WMZOGYVENU5Y32EXPWB5NY7SNTSQAN4L47JWL
               3SCBZOLLAFXTZ6JK2RASOSQAL43SXXBA7ALY33GZ4R6QTXCN2RKXEPYRJ4PSJ34
               NEEL2XOAWXID2XQRJPDV3EHRZSDSKMAM

The text form is one string of 284 characters; it is wrapped here for the
page, and the parser ignores the line breaks.

Contact card of I, as it appears in message 3 (171 bytes): epoch 1, one
endpoint, no invitation.

    013d4017c3e843895a92b70aa74d1b7ebc9c982ccf2ec4968cc0cd55f12af466
    0cde9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b
    4f000000000000000101ed4928c628d1c2c6eae90338905995612959273a5c63
    f93636c14614ac8737d1007c98b9a384ab4a8dd22e7b752249ae282a5d4072de
    c9c640f2d1d59de1844e5f3192e3263077fd11c77b5afb9f9b80d2e2d77b1648
    7b43119b9746b95f2e2f0d

Handshake (section 4). I dials R. The ephemeral secret keys are fixed for
this vector:

    I ephemeral secret  1111111111111111111111111111111111111111111111111111111111111111
    I ephemeral         7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13
    R ephemeral secret  2222222222222222222222222222222222222222222222222222222222222222
    R ephemeral         0faa684ed28867b97f4a6a2dee5df8ce974e76b7018e3f22a1c4cf2678570f20

    prologue (51)
               4d4f4e4f4c4954482d53455353494f4e2d5631
               d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a

    message 1 (48)
               7b4e909bbe7ffe44c465a220037d608ee35897d31ef972f07f74892cb0f73f13
               c4613ae914614af97813408bce932efc

    message 2 (48)
               0faa684ed28867b97f4a6a2dee5df8ce974e76b7018e3f22a1c4cf2678570f20
               04bdc426ae5793c3d04345705141dfbc

    message 3 (235)
               30daabb09afb8c0ac3ab7397a17abdddbd1e51bb900dde33066249b26217d6e2
               9d6ab023301792b938bc608f195d252b8ab93333f1347d12417c36b28a88709f
               9bd9c35fc186e3137e0cf5821cc0fa8ecfed61861537004d0d158d40876abf47
               5886d070d79c147a6fdd424de135041ea7fb24b233841dc38611edb24427d4c5
               77d3025b18eba9a5467abb80bc931570f1055817fa7f4ab92d030b47fd845194
               3b221b2891bbd72e367b943a235f4972838c034bff66d46487e07c51fdb23786
               77a37b6f62792193522f75c3b4eb9aec237f8daf83e8ae9341f681ed2a618802
               db7d3684d5fc2968897ac9

    handshake hash h after message 3
               ef8f280b4bc4ccc6f2e5164984de6738f1b467a970841480f6ebdf61a69716a9

Frames (section 5). The first frame each side sends is a ContactAccept:
plaintext `00 11 00 00` and 1020 zero bytes, 1042 bytes on the wire with
the length prefix `04 10`. SHA-256 of those 1042 bytes:

    first frame of I    b11fcacca95b6d25e038cc84eadeea2ede1e5e881fb6aa3a6138f743ec659785
    first frame of R    d2e82f5edebfb960a8ef4e58c29889a741c481f0990ad048662965c612c7fb19
    second frame of I,
    same plaintext      990ee883b76ee5ed77d00281c72569765ad8afdccd36d2736b629fa47cdf7c5a

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

X25519 key validity (section 10.2): the five strings listed there are
rejected; so is any string whose value is 2^255 - 19 or more, for example
`ed` followed by 30 bytes `ff` and `7f`, and any string with the top bit
set; the transport keys above are accepted.

### 16.2 Vectors still to be produced

- onion address derivation from a key, with the Tor backend;
- frame encoding of every message type at minimum and maximum size.

## 17. Open questions

P1. Padding block size. 1024 bytes is a judgment call between overhead and
    how much length information leaks. Needs review together with the
    traffic-analysis limits in the threat model.

P2. Whether a session should also be bound to the onion service key the
    connection was made to, by adding that key to the prologue. It would
    tie the session to the endpoint as well as the identity. It
    complicates endpoint migration, when a responder serves two endpoints
    for a while and has to know which one a stream arrived at. Currently
    not bound. A party that only forwards bytes between an initiator and
    the real responder is therefore not detected; it sees ciphertext
    only, and any change it makes ends the session.

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

P8. A period in which an identity answers handshakes for both its previous
    and its new transport key, so that contacts with the older card are
    not cut off (section 11.4). A responder would have to try message 1
    against two keys. Not in version 1.

P9. Whether the card in a ContactRequest is still needed, now that the
    initiator presents its card in the handshake. It is kept so that a
    request is complete in itself when it is queued, and required to be
    consistent with the handshake (section 8.3).

P10. Which capability a request carries when the user holds several cards
    of one identity. A request carries the capability of the card held of
    the peer (section 8.3). A second card of the same epoch that differs
    only in its capability is not a newer statement (section 11.4), so the
    rules for what is held do not replace the first. A user who was given
    a new card because the old capability was revoked needs the new one
    to be used. Proposed: a card the user imports by hand for a peer that
    is not an accepted contact replaces the held card when it states the
    same keys, endpoints and epoch; the session layer already takes the
    capability from whatever card it is given. Decided with the contact
    store in Phase 4.

P11. Requests in the queue when the capability they carried is revoked.
    Section 12.3 keeps them, because revocation decides about new
    requests only. Revoking a capability that leaked is also the moment a
    user may want to discard what it brought in. Proposed: keep them, and
    let the interface offer to decline them together, as a separate user
    action. Decided with the contact store in Phase 4.
