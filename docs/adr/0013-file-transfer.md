# ADR 0013: Resumable file transfer

Status: decided for Phase 8, after independent reviews of the design.
Date: 2026-10-24.

## Context

Version 1 of the protocol (`PROTOCOL.md` sections 8.9 and 13) defines six
file messages and an exchange in which a transfer lives and dies with one
session: "A session that ends aborts its transfers. There is no resume."
No release has implemented that exchange. Since Phase 7 Monolith answers
every FileOffer at once with FileReject, remembers its identifier for the
session, discards a file message that names a remembered identifier and
treats any other as a violation (`DESIGN_QUESTIONS.md` P7-4).

Phase 8 has to send a file to a contact, with explicit acceptance, over
the authenticated session and Tor only, and the transfer has to survive
the loss of a session, a restart of either side and a restart of Tor.
Facts of the code that shape the design:

- A session ends after `IDLE_TIMEOUT` (240 s) without a frame received.
  In the exchange of version 1 the receiver sends nothing between
  FileAccept and FileComplete, so an upload of a few minutes would end
  the sender's session.
- Between the session task and the peer's reader lie the socket to Tor,
  Tor's buffers and the stream window: easily a megabyte. Writing "at
  most one chunk ahead of chat" orders the writes but does not bound how
  long a chat message or an acknowledgement waits behind the chunks
  already written; at 20 KB/s a megabyte is longer than
  `MESSAGE_ACK_TIMEOUT` (60 s).
- `SESSION_LINGER` (120 s) closes a session that carries no chat, and
  the reconnect scheduler dials only for queued chat messages; a session
  that lasted `RECONNECT_RESET_AFTER` resets its delays.
- A later protocol version, or any new message type, needs a new label
  in the handshake prologue (sections 3, 4.2 and 6.3). Peers with
  different labels cannot complete a handshake at all, so a new label
  would cut chat between a Phase 8 peer and a Phase 7 peer.

## Decision

### The label stays `MONOLITH-SESSION-V1`; section 13 is amended

The file exchange of version 1 is amended in place. No message type is
added and no label changes. One layout changes: FileAccept gains an
offset.

    FileOffer     [16] transfer_id || u64 size || text<1..255> filename
    FileAccept    [16] transfer_id || u64 offset
    FileReject    [16] transfer_id
    FileChunk     [16] transfer_id || bytes<1..64490> data
    FileComplete  [16] transfer_id || [32] sha256
    FileAbort     [16] transfer_id

This is compatible with every peer that exists:

- No implementation of the earlier exchange exists.
- A Phase 7 peer receives only FileOffer and FileAbort from a Phase 8
  peer, both in their version 1 layout, and Ping, which it answers. It
  answers each offer with FileReject, and the Phase 8 sender marks the
  transfer rejected and never offers it again. A FileAbort that follows
  an offer on the same session names an identifier the Phase 7 peer
  remembers, and it is discarded.
- A Phase 7 peer never offers, so it never receives FileAccept,
  FileChunk or FileComplete, the messages whose meaning changes.
- A Phase 8 peer writes a file message only on a session on which the
  transfer was offered, and offers a transfer at most once per session
  (below), so a Phase 7 peer never sees an identifier it does not know or
  a second offer of one it knows.

FileOffer keeps its layout because a Phase 7 peer decodes it: a longer
offer, with a digest in it, would end that peer's session and with it
the chat. The digest stays in FileComplete, as in version 1. It is the
SHA-256 of the `size` bytes of the file.

### Identity of a transfer

On the wire a transfer is the 16 random bytes `transfer_id` the offerer
drew. Locally a transfer is the key (local identity, contact record,
direction, `transfer_id`): `IdentityId` and `ContactId` are the durable
identifiers of ADR 0008, which are never reused, so a contact deleted or
blocked and made again for the same person gets a new `ContactId` and
inherits nothing; a receiver that made a contact again sees a repeated
offer as a new one and must accept it again. The same 16 bytes from two
contacts are two transfers. An outgoing identifier is drawn with
`getrandom` and drawn again if the local store holds it for that contact
in either direction; an offer from the peer that names an identifier
this side offered on the same session is a violation.

### Names

The name of an offer is display data, never a path (section 9). A local
file is offered under its last path component, made valid: each
character the filename rules reject is replaced with `_`, spaces at the
ends are removed, the result is cut to 255 bytes on a character
boundary, and a result that is empty, `.` or `..` becomes `file`. The
sender sees the name as it will be offered before it offers. The
receiver never builds a path from a name; a suggestion for saving comes
from `Filename::save_name`, in the folder the user chose (ADR 0014).

### Transfers outlive sessions

A transfer is durable state of the local store (ADR 0014), not of a
session. Each session keeps the set of identifiers offered on it, by
either side, and the exchange of each transfer starts again on every
session from what the two stores hold:

1. The offerer offers each transfer it has not finished, at most once
   per session, with the same identifier, size and name every time, on
   a session that is confirmed, is its preferred session for the contact
   and is the only session it has with the contact (ADR 0009); while a
   duplicate is being resolved it offers nothing. It offers first the
   transfers being sent (few: the receiver accepts no more than its own
   active limits allow), then those it cancelled while
   the receiver could not be told, each followed at once by FileAbort,
   then those waiting for an answer, oldest first, at the pace of
   `FILE_OFFER_RATE` and after any chat waiting to be written. A transfer
   cancelled before it was ever offered is not offered.
2. The receiver answers an offer from what it holds for that key:
   - nothing: a new offer (below);
   - an offer the user has not decided, or a transfer it holds after a
     failed write (ADR 0014): nothing;
   - accepted and not complete: `FileAccept(id, offset)`, where `offset`
     is the number of bytes it holds durably;
   - complete: `FileComplete(id, digest)` with the digest it verified,
     also once the file was saved or removed;
   - rejected or expired: FileReject;
   - cancelled or failed: FileAbort.
   An offer whose size or name differs from the record of that key is a
   violation.
3. After `FileAccept(id, offset)` the offerer sends the bytes from
   `offset` on, then FileComplete, as below.

The receiver binds a transfer to a session when the transfer is offered
on it and is not bound, and moves the binding only to a session that
was registered later than the bound one. When the bound session ends,
the binding passes to the most recently registered live session on which
the transfer was offered, if there is one, and the receiver writes its
answer to that offer there if it has not yet. When the binding moves
away from a session, the transfer
counts there as ended, with at most `FILE_WINDOW_CHUNKS` chunks still
expected; file messages for it on that session are discarded within
that bound. Writes to a blob and the offset taken from it are serialized
per transfer. A decision of the user goes to the bound session, or waits
for the next offer if there is none. The offerer sends file data only on
the session of its latest offer of the transfer and only while that
session is its preferred one.

A side makes the change of state that a file message causes durable
before it reads the next frame, as it does for a chat message it takes
in, and reads no frame while it writes a chunk or verifies a file. The
data of a chunk is the exception: it becomes durable only before a
FileAccept counts it.

Within a session the rules of version 1 hold: a file message for an
identifier not offered on the session is a violation, a second FileOffer
of an identifier already offered on it is a violation, and each side
remembers the last `MAX_ENDED_TRANSFER_IDS` identifiers that ended on the
session, with the bytes still expected for each, and discards file
messages that name them; a chunk beyond the bytes still expected is a
violation. Discarded messages are not activity of the session.

### Knowing that the peer read a message

A frame that was written may never be read: the session can end with it
in a buffer. Where that matters, the side that wrote it writes a Ping
after it and takes the Pong that answers that Ping (section 8.2) as proof
that the peer read everything before it, frames being read in order. A
Pong of a Ping written earlier proves nothing about later frames; while
such a Ping is outstanding the barrier waits for its Pong and then
writes its own Ping. This is used:

- by the offerer, to record that an offer was read (flag bit 0 of ADR
  0014), and that a cancellation was read;
- by the receiver, to record that the first FileAccept after the user
  accepted, a FileAbort, or the final answer to a transfer (FileComplete,
  FileReject) was read.

Until the Pong comes, the record says that the peer is not known to have
read it (flag bit 1). Since a side makes the state a file message causes
durable before it reads the next frame (above), a Pong also means the
peer recorded it. A Phase 7 peer answers Pings, so this needs nothing new
from it. An offer read but answered with nothing (the receiver held it
back for its limits, or holds the transfer after a failed write) gets
bit 0 all the same: it then waits for a session made for another reason,
for the receiver to dial, or for `TRANSFER_STALL_LIMIT`.

### Offers and acceptance

- An offer above `MAX_TRANSFER_SIZE`, or above
  `MAX_PENDING_FILE_OFFERS_PER_CONTACT`, is answered with FileReject and
  leaves no record: the same offer gets the same answer every time.
- An offer that would exceed `MAX_PENDING_FILE_OFFERS` of the local
  identity, `MAX_TRANSFER_RECORDS_PER_CONTACT` or `MAX_TRANSFER_RECORDS`
  is answered with nothing and leaves no record; the offerer repeats it
  on a later session.
- Otherwise the offer is recorded, durably, and shown to the user, who
  accepts or rejects it. Nothing of the file is stored or allocated
  before acceptance; no local path is opened.
- An offer the user has not decided after `FILE_OFFER_TIMEOUT` expires:
  its record says so, FileReject is written on the bound session if
  there is one, and the next offer of it is answered with FileReject.
- Acceptance reserves the declared size against
  `MAX_STAGED_INCOMING_BYTES`, needs a free place among the active
  transfers, and reserves the whole blob on disk with
  `FILE_FREE_SPACE_MARGIN` left over (ADR 0014). If any is missing it is
  refused and the offer stays pending; the interface says why. On
  acceptance the receiver creates the blob and records the transfer as
  accepted, durably (ADR 0014), and writes `FileAccept(id, 0)` on the
  bound session, or on the next offer of it.
- A FileAccept or FileReject that the offerer receives must name an
  offer the offerer made on this session.

### Data, window and progress

- `FileAccept(id, offset)`: the receiver has accepted the transfer and
  holds its first `offset` bytes durably. `offset` is `min(n * L, size)`
  for a number n of whole chunks, L the full chunk size (64490 bytes for
  the parameters of version 1). Within a session the offsets the receiver
  sends for one transfer never decrease.
- FileChunk is valid only from the offerer and only after a FileAccept
  of the transfer on the session. The first chunk of a session starts at
  the offset of that FileAccept, the chunks follow each other with no
  gap and no overlap, and each carries exactly `min(L, bytes remaining)`
  bytes.
- The offerer never has more than `FILE_WINDOW_CHUNKS` chunks written
  beyond the last offset the receiver acknowledged on the session. With
  the window full it writes no chunk until a FileAccept moves the
  offset. The window bounds how long a chat message, a MessageAck or a
  Ping written by the offerer waits behind its chunks: the window over
  the slowest throughput the design supports, `MIN_TRANSFER_THROUGHPUT`
  (10 KB/s), stays below half of `MESSAGE_ACK_TIMEOUT`. Its side effects
  are accepted: a transfer goes no faster than about the window per
  round trip and sync, some hundreds of KB/s on onion circuits; below the
  floor a chat acknowledgement can time out and end the session, which
  the transfer survives; and below about 25 KB/s a duplicate probe can
  wait behind the window past its timeout and close the session it
  probes, after which the transfer goes on over the other one.
- The window binds the offerer alone. The receiver does not end a
  session for a chunk beyond it: it cannot know which of its
  acknowledgements the offerer had read when it wrote the chunk, so a
  check against the last FileAccept it wrote would catch some breaches
  and prove none. What the receiver checks bounds what an offerer that
  ignores the window gets: the length and place of every chunk, nothing
  beyond the size (a violation), nothing staged beyond the blob reserved
  at acceptance, and each chunk written before the next frame is read,
  so that the receiver's own writes go out between chunks. Such an
  offerer gains speed in its own direction of the session and nothing
  else. A way to exhaust a resource through it would reopen this.
- A FileChunk carries no offset: its place is the offset of the
  FileAccept that opened the window on the session plus the length of
  the chunks before it. An offerer that resumes from another offset than
  the one it was given writes chunks of the right length in the wrong
  place, and the receiver cannot tell until the digest does not match at
  FileComplete (below). The transfer then fails and nothing is taken as
  complete, but only after the rest of the file was sent, staged and
  hashed: the cost of such a fault is bandwidth and work, never a wrong
  file.
- The receiver writes each chunk before it reads the next frame. Once
  `FILE_PROGRESS_CHUNKS` more chunks are durable, or
  `FILE_PROGRESS_INTERVAL` has passed since its last FileAccept and new
  data is durable, it syncs the blob and writes `FileAccept(id, offset)`
  again. These acknowledgements move the offerer's window, keep its
  session from its idle timeout, and tell it how far the receiver is; the
  offerer never learns more than that. An offset above the bytes the
  offerer sent is a violation.
- The offerer writes `FileComplete(id, digest)` once per session per
  transfer: after the last chunk, or at once if the FileAccept that
  answered its offer on this session has `offset` equal to `size` (a
  receiver that holds all bytes but has not verified them, or a file of
  size zero). A FileAccept with `offset` equal to `size` that arrives
  later on the session asks for nothing. FileComplete from the offerer is
  valid only when the receiver holds exactly `size` bytes of the
  transfer; one that comes with bytes missing is a violation. After it
  the transfer has ended on that session for the receiver, so a further
  file message of it there is discarded as for any ended identifier.

### Completion

The receiver, on FileComplete:

1. checks that it holds exactly `size` bytes;
2. computes the SHA-256 of the `size` bytes it staged and compares it
   with the digest it received;
3. on a match records the transfer as complete, with the digest, and
   waits until that record is durable;
4. only then writes `FileComplete(id, digest)` back.

On a mismatch it records the transfer as failed, durably, then removes
what it staged and writes FileAbort; it never offers the bytes as a
received file and never asks for them again. A segment that fails to
authenticate during the verification is neither: it and every segment
after it count as not written, the receiver ends the session with Close
(an offset it sent there cannot go down), and answers the next offer
with the lower offset. Complete, for the
receiver, means that the whole declared byte sequence is staged durably
and verified; it does not mean that the user saved, opened or read it.

The offerer, on FileComplete from the receiver, waiting or sending (it
may have crashed before it recorded that the transfer was accepted),
compares the digest with the one it staged. On a match it records the
transfer as delivered, durably, and removes its staged copy; on a
mismatch it records the transfer as failed and writes FileAbort.
Delivered, for the offerer, means that it holds an authenticated
acknowledgement of that exact transfer.

A crash between the steps repeats a step and never makes a second
transfer: a receiver that verified but did not yet record the completion
answers the next offer with `FileAccept(id, size)`, receives FileComplete
again and verifies again; one that recorded it answers with
FileComplete; an offerer that did not record the acknowledgement offers
again and receives it again.

### The offerer's table

| Offerer holds | Receives | Then |
| --- | --- | --- |
| waiting | FileAccept answering its offer, a valid offset | sending (recorded once); window starts at the offset |
| sending | FileAccept answering its offer on a new session, a valid offset, which may be lower than on an earlier session | window starts at the offset; nothing recorded |
| sending | FileAccept, a later offset on the same session | window moves; progress per the records section |
| waiting or sending | FileComplete, digest matches | delivered |
| waiting or sending | FileComplete, digest differs | failed; FileAbort |
| waiting or sending | FileReject | rejected (also when the peer lost its record or runs an earlier version) |
| waiting or sending | FileAbort | cancelled by the peer |
| cancelled | FileComplete, digest matches, on any session | delivered: the receiver holds the file |
| any ended state | another file message of the transfer on the session | discarded, as for an ended identifier |

The offerer never refuses an acceptance because of its own limits: the
limits of active transfers bind the side that accepts.

### The receiver's table

| Receiver holds | Receives | Then |
| --- | --- | --- |
| nothing | FileOffer | a new offer (above) |
| offered | FileOffer again, on a later session | nothing; the deadline stays |
| any | FileOffer of an identifier already offered on this session | violation |
| any | FileAccept or FileReject for an offer the peer made | violation |
| offered | FileAbort | cancelled by the peer; the offer disappears from the interface |
| accepted | FileOffer | `FileAccept(id, offset)` |
| accepted, bound to this session | FileChunk at the expected offset | written, then progress |
| accepted, bound to another session | FileChunk | discarded within the bound left where the binding moved away; beyond it, violation |
| accepted, bound to this session | FileComplete with all bytes held | verification (above) |
| accepted | FileAbort | cancelled by the peer; blob removed |
| complete | FileOffer | `FileComplete(id, digest)` |
| complete | FileAbort | discarded |
| rejected or expired | FileOffer | FileReject |
| cancelled or failed | FileOffer | FileAbort |
| any | FileChunk out of order, beyond the size or before acceptance | violation |

### Cancellation and rejection

- Either side may cancel a transfer that is not complete; for the
  receiver a FileAbort for a complete transfer is discarded. The record
  becomes cancelled, durably, its staged data is removed, and FileAbort
  is written if the transfer was offered on a live session; otherwise
  the offerer writes it after its next offer (above) and the receiver in
  answer to the next offer. In both cases the record says that the peer
  was not told until a Pong after the FileAbort says it was. Cancelling
  twice is cancelling once.
- A rejected or cancelled transfer is never offered again once the
  offerer holds the answer, and is answered with FileReject or FileAbort
  by the receiver; a repeated offer of it shows nothing new and allocates
  nothing.

### Records that make repetition safe

- The receiver keeps the record of every ended incoming transfer (
  rejected, expired, cancelled, failed, complete and saved, complete and
  removed by the user) for `ENDED_TRANSFER_RETENTION`; removing a
  complete transfer from the interface removes its blob and hides its
  record, which stays for that time so that a repeated offer finds it.
- Progress of a transfer is a change of its state, or a durable offset
  that grew by at least `TRANSFER_PROGRESS_BYTES`, or reached `size`,
  since the progress counted last. An offer repeated, or an offset
  acknowledged again, is not progress. A contact can thus reset the
  delays of the scheduler at most `size / TRANSFER_PROGRESS_BYTES` times
  per transfer, 64 for the largest file. Each record that can stall
  (outgoing waiting, sending, and cancelled not yet read; incoming
  accepted) keeps in its `deadline` the time of its last progress plus
  `TRANSFER_STALL_LIMIT`; the store is written for it at most once per
  `TRANSFER_PROGRESS_RECORD_INTERVAL` per transfer, so the deadline
  survives restarts and costs few writes.
- An offerer whose transfer passes its deadline fails it and stops
  offering it; for a cancellation not yet read it stops trying to tell
  it. `TRANSFER_STALL_LIMIT` is below `ENDED_TRANSFER_RETENTION`, which
  starts when the receiver ended its record, after any progress the
  offerer could count, so a receiver still holds the record of any
  transfer an offerer may still offer.
- A receiver fails an accepted transfer that passes its deadline, or
  that the offerer did not offer on `TRANSFER_UNOFFERED_SESSIONS`
  sessions in a row that were confirmed and stayed the only session of
  the contact for at least `TRANSFER_GRACE` of the receiver's reading
  time, or that the peer closed with Close while they were the only
  confirmed session: the offerer no longer has it. An offer counts as
  made on a session whatever the answer to it, discarded included.
  Failing it ends its claim on the scheduler and removes its staged data,
  and needs no telling: it sets no flag that the peer has not read it.
- A cancellation of the receiver that the offerer has not read stops
  claiming the scheduler by the same rule, or `TRANSFER_STALL_LIMIT`
  after it was made; its record stays, with its flag, until its
  retention ends.
- A record that says the peer is not known to have read a message, and a
  record of a transfer that is not ended, are never removed to make room.
- Each contact holds at most `MAX_TRANSFER_RECORDS_PER_CONTACT` records,
  the installation `MAX_TRANSFER_RECORDS`. To make room the store removes,
  oldest first: an ended outgoing record, of any age; then an ended
  incoming record that ended more than `TRANSFER_STALL_LIMIT` plus a day
  ago, when no offerer can still offer it; then an ended incoming record
  of any age whose final answer the peer is known to have read, which
  the offerer will therefore not offer again. If nothing can go, an offer is answered
  with nothing and leaves no record, and a file the user would send is
  refused with a message that the list of transfers is full.

### The contact and the identity

- Blocking or deleting a contact ends its sessions (as before) and
  cancels every transfer of that contact record. Nothing is announced to
  the peer. The staged data of those transfers is removed, and so are
  their records with the record of a deleted contact.
- A rotation of the peer's key keeps the contact record, so its
  transfers go on over the session of the new key, offered there as on
  any new session.
- Closing an identity stops its transfers; results of work that was in
  flight are dropped by the same checks that drop them for messages.
  Deleting an identity removes its transfers and their staged data.

### Scheduling

- Within a session the order of writes is: replies to what was read
  (Pong, MessageAck, file answers, including FileAccept progress), then
  the head of the chat queue, then decisions and cancellations of the
  user, then offers, at their pace, then at most one FileChunk. Before it
  writes a chunk the session task takes every complete frame that has
  arrived without waiting for one that has not, so with transfers in
  both directions neither side stops reading. A waiting chat or control
  message is written after at most one chunk and waits behind at most
  the window. One transfer per contact sends at a time, the oldest
  accepted first.
- A session is not idle while a transfer on it made progress (a chunk
  read or written, an offset acknowledged) within the last
  `SESSION_LINGER`; after that `SESSION_LINGER` applies as for chat.
- The reconnect scheduler of a contact also runs while a transfer needs
  a session: on the offerer, an offer not yet read, a transfer being
  sent, and a cancellation not yet read; on the receiver, a transfer
  accepted and not complete, and a cancellation of its own not yet read.
  An offer waiting for the user's decision, a rejection or an expiry not
  yet read (the offerer learns them at its next offer), a final answer
  not yet read, and a transfer held after a failed write make no dial on
  either side. Cancelling or failing a transfer removes its claim.
- A dial for transfers alone resets the scheduler's delays only if the
  session it made brought progress, as defined above, to one of them;
  otherwise the delays keep growing, as after an acknowledgement
  timeout. A contact that accepts and then never sends, sends one chunk
  per session, or never answers, cannot make this side dial it every few
  minutes, which would tell it when this side is online.
- A session ends at `MAX_SESSION_LIFETIME` whatever its transfers do;
  they go on over the next session. Phase 8 does not use the longer
  lifetime of version 1 for a session with a transfer, which section 7.1
  allows as local policy. A duplicate session that loses (section 14)
  ends; the transfers on it go on over the one that stays.

### Limits

New or changed, all in `monolith-protocol` `limits.rs` and
`RESOURCE_LIMITS.md`:

| Name | Value | Why |
| --- | --- | --- |
| `MAX_TRANSFER_SIZE` | 256 MiB | Local limit of a file sent or accepted (replaces `DEFAULT_MAX_FILE_SIZE`). The staged copy is a second copy on disk, and an ephemeral installation stages in memory; the owner capped Phase 8 at 256 MiB. `MAX_FILE_SIZE` (4 GiB) stays the largest size an offer may declare on the wire. |
| `FILE_OFFER_TIMEOUT` | 24 hours | Was 10 minutes, for offers that died with their session. An offer now waits for the user across sessions and restarts. |
| `FILE_WINDOW_CHUNKS` | 4 | About 252 KiB written beyond the acknowledged offset; at 10 KB/s a chat message waits at most about 26 s behind it. |
| `FILE_PROGRESS_CHUNKS` | 2 | Half the window, so the window moves before it fills. |
| `FILE_PROGRESS_INTERVAL` | 30 s | Longest time between two acknowledgements while data arrives; well below `IDLE_TIMEOUT`. |
| `TRANSFER_STALL_LIMIT` | 7 days | A transfer that made no progress that long fails on either side. |
| `ENDED_TRANSFER_RETENTION` | 14 days | How long the record of an ended incoming transfer is kept; above `TRANSFER_STALL_LIMIT`. |
| `TRANSFER_PROGRESS_RECORD_INTERVAL` | 1 hour | At most one write of the store per transfer for its stall deadline. |
| `TRANSFER_UNOFFERED_SESSIONS` | 3 | Sessions without an offer after which a receiver fails an accepted transfer. |
| `TRANSFER_GRACE` | 60 s | Reading time a session must have spent as the only one of the contact to count: the offers of the transfers being sent come first, within the burst of the rate, after at most a duplicate probe (10 s). Below `SESSION_LINGER`, so an idle session can count. |
| `TRANSFER_PROGRESS_BYTES` | 4 MiB | Growth of a durable offset that counts as progress. |
| `MIN_TRANSFER_THROUGHPUT` | 10 KB/s | The floor the window is sized for. |
| `TRANSFER_HOLD` | 10 min | How long a transfer is held after a write failed for a full disk or a held file (ADR 0014). |
| `MAX_WAITING_OFFERS_PER_CONTACT` | 4 | Outgoing transfers not yet accepted, cancellations not yet told included, per contact. |
| `MAX_WAITING_OFFERS` | 16 | The same per local identity. |
| `MAX_ACTIVE_TRANSFERS_PER_CONTACT` | 2 | Incoming transfers accepted and not complete, per contact. |
| `MAX_ACTIVE_TRANSFERS` | 4 | The same per local identity. |
| `MAX_STAGED_OUTGOING_BYTES` | 1 GiB | Declared sizes of outgoing transfers not finished, per installation. |
| `MAX_STAGED_INCOMING_BYTES` | 1 GiB | Declared sizes of incoming transfers accepted, complete ones not yet saved or removed included, per installation. |
| `MAX_EPHEMERAL_STAGED_BYTES` | 256 MiB | Both directions together, in memory, for an ephemeral installation. |
| `MAX_TRANSFER_RECORDS_PER_CONTACT` | 16 | Records of one contact, ended ones included. |
| `MAX_TRANSFER_RECORDS` | 256 | Records of the installation. |
| `MAX_STAGING_JOBS` | 1 | Files staged at a time; one save at a time. |
| `FILE_FREE_SPACE_MARGIN` | 64 MiB, unchanged | Space left free by every reservation; at least what the stores need to be written once more (16 + 1 + 16 MiB). |

`MAX_PENDING_FILE_OFFERS_PER_CONTACT` (4) and `MAX_PENDING_FILE_OFFERS`
(16) keep their values and now count per local identity. An accepted
transfer is never removed to make room for another.

### Rates

The rates of Phase 7 stay. FileOffer counts against `FILE_OFFER_RATE`,
the other file messages but FileChunk against `CONTROL_MESSAGE_RATE`.
FileChunk is bounded by the declared size of an accepted transfer and by
the receiver, which reads no frame while it writes, and discarded chunks
by the bytes still expected. A contact can make the receiver decrypt and
hash at most what it declared and the receiver accepted, once per
transfer at completion, plus one pass over an accepted blob after each
start of the installation (ADR 0014); a digest mismatch fails the
transfer for good.

## Consequences

- `PROTOCOL.md` changes: section 8 (the body size of FileAccept, 24),
  8.9, 13 (the exchange, the window that replaces "no application-level
  window" of 13.2, the receiver's handling of 13.3), 14 (the transfers of
  a losing session go on over the other), 7.1 (Phase 8 does not use the
  longer lifetime) and 8.2 (Ping as a barrier). The label does not.
  ADR 0009 says that a Pong may answer a barrier.
  `RESOURCE_LIMITS.md`, `SECURITY_INVARIANTS.md` (S13, S14),
  `THREAT_MODEL.md`, `STORAGE.md` and `ARCHITECTURE.md` follow.
- The test of Phase 7 that a later session treats an identifier of an
  earlier one as a violation changes with it: a later session now knows
  the transfers of the store.
- The core gets a transfer module with a state machine apart from the
  session task, durable records (ADR 0014), the chunk pump with its
  window in the session task, the barrier, and the scheduler and linger
  rules above. The front end gets commands to offer, accept, reject,
  cancel, remove and save a transfer, events that say that a transfer
  changed, and queries that give its state; events are notifications,
  the queries the truth.
- An offerer learns nothing of the receiver's progress beyond the
  acknowledged offsets, and the receiver nothing of the offerer's file
  beyond its size, name and digest.

## Alternatives considered

- A new protocol label with a resume message: cuts chat with every Phase
  7 peer, which the brief of Phase 8 rules out.
- A new message type within version 1: a Phase 7 peer that received it
  would end the session. Changing FileAccept reaches only peers that
  accept, which Phase 7 peers never do.
- The digest in FileOffer: a Phase 7 peer could not decode the offer and
  would end the session, chat included.
- Resume by sending again from the start and skipping what the receiver
  holds: correct, but it sends the whole file again over Tor after every
  lost session.
- No window, as in version 1: chunks buffered in Tor delay chat and
  acknowledgements past their timeouts.
- No resume, as in version 1: fails the brief, and an upload of more than
  a few minutes would rarely finish on Tor.

## Changes during the implementation

- What a front end is shown of a transfer is what the transfer store
  holds durably (ADR 0014, "Changes during the implementation"): a
  completion, a cancellation or an offer recorded in memory is not
  listed until its write completed. The model test of the transfers
  found that a crash could otherwise show a cancelled transfer offered
  again, or a received file receiving again.
- A file message that tells the peer something the state decided (an
  offer, FileAccept, FileReject or FileAbort) is written only once every
  change applied so far is durable (`Transfers::settled`). Before, a
  session could write an offer, a decision or a cancellation from the
  state in memory, and a crash could leave the peer with a message the
  other side no longer held. Crash tests at every storage step of an
  offer, of each decision of the receiver and of a cancellation of the
  sender check it.
- The user may remove a cancelled outgoing transfer whose cancellation
  the receiver has not read yet: its record goes, and the receiver learns
  that the file is no longer offered (an accepted transfer fails as
  `OffererGone`, an undecided offer expires). Before, the interface
  offered the removal and the core refused it.
- An offerer writes a chunk only after an acceptance on the same
  session, from the offset that acceptance gave. A FileChunk carries no
  offset, so a sender that broke this rule would stage bytes at another
  offset than it meant; the receiver would find it only at the
  completion, as a digest that differs, and the transfer would fail.
- What the completion proves is the integrity of the transfer: the file
  is the one its sender declared, every byte, with that digest. It says
  nothing about whether the content is safe. Monolith never opens,
  previews or runs a received file.
