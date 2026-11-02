# ADR 0014: Encrypted staging of transfers

Status: decided for Phase 8, after independent reviews of the design.
Date: 2026-10-24.

## Context

ADR 0013 makes a transfer durable state that outlives sessions and
restarts. The sender stages the exact bytes it offers before the offer
exists; the receiver stages what it accepts until it is verified and
saved. A persistent installation keeps nothing in plaintext at rest
(ADR 0005, `STORAGE.md`), and an ephemeral one writes nothing to disk at
all (S25). Files may be up to `MAX_TRANSFER_SIZE` (256 MiB), so neither
side may hold a whole file in memory, and the message store, one sealed
file of at most 16 MiB rewritten whole (ADR 0008), cannot hold them.

What exists to build on:

- XChaCha20-Poly1305 from `chacha20poly1305`, HKDF-SHA256 from `hkdf`,
  SHA-256 from `sha2` and `getrandom`, all already used by
  `monolith-storage` (`CRYPTOGRAPHY.md`); no other primitive is needed.
- The vault key, 32 random bytes held by the open vault; the message
  store key is derived from it with HKDF at create and open time
  (`vault.rs`), before the vault moves into the write job.
- The profile directory with its lock, its exclusive-create writes, its
  directory syncs and, on Windows, its bounded retry of renames and
  removals (P7-10). `VaultDir` reads and writes whole files only.

`CRYPTOGRAPHY.md` says Monolith defines no primitive. This ADR defines a
format, not a primitive: each segment is an ordinary AEAD message, and
the format fixes what goes into its key, nonce and associated data.

## Decision

### Files

In the profile directory, beside `vault` and `messages`:

- `transfers` (and `transfers.new` while it is written): the transfer
  store, the records of every transfer of the installation;
- `transfer-` followed by exactly 32 lower-case hexadecimal digits: the
  staged data of one transfer, named by its blob identifier (below),
  never by anything the peer sent. A name is recognized only if it
  matches that pattern exactly; `transfers` and `transfers.new` are not
  blobs.

Every file is created with exclusive-create semantics, mode 0600 on
Unix, without following a link or a reparse point, in the directory the
installation holds locked, and opened again the same way, with the
owner and mode checks of the other files. Nothing else is written for
transfers. An ephemeral installation keeps the same structures in
memory (below).

### Keys

Derived from the vault key at create and open time, as the message store
key is, and dropped (zeroized) with the installation:

    store_key   = HKDF-SHA256(vault_key, salt = none,
                              info = "MONOLITH-TRANSFER-STORE-V1")
    staging_key = HKDF-SHA256(vault_key, salt = none,
                              info = "MONOLITH-TRANSFER-STAGING-V1")

and for each blob:

    blob_key = HKDF-SHA256(staging_key, salt = none,
                           info = "MONOLITH-TRANSFER-BLOB-V1" || blob_id)

`blob_id` is 16 bytes from `getrandom`, drawn when the blob is created,
never zero, and drawn again if a blob of that name exists or a record
already names it. The key is always derived from the `blob_id` of the
record, never from bytes read from the blob. A blob is never written
under another key. A change of passphrase rewraps the same vault key, so
staged blobs stay readable; a rekeying of the vault, not implemented yet
(`STORAGE.md` section 3.3), would have to discard them.

### The transfer store

The same construction as the message store (ADR 0008), with its own
magic and key:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `MNLTXFR\0` |
| 8 | 2 | format version, 1 |
| 10 | 2 | reserved, 0 |
| 12 | 8 | generation |
| 20 | 24 | nonce, random for every write |
| 44 | 4 | payload length, tag included |
| 48 | n | XChaCha20-Poly1305(store_key, nonce, payload, ad = bytes 0..48) |

The file is at most `MAX_TRANSFER_STORE_FILE_LEN` (1 MiB). It is written
whole, as the message store is: sealed with the next generation, written
to `transfers.new` with exclusive create and a sync, renamed over
`transfers`, and the directory synced; at open the recovery of
`STORAGE.md` section 3.5 applies to it unchanged, with the names
`transfers` and `transfers.new`, the exception of ADR 0008 for a
`.new` alone that does not open included. It is written when the state
of a transfer changes, and for the stall deadline of a transfer at most
once per `TRANSFER_PROGRESS_RECORD_INTERVAL` (ADR 0013); never for each
chunk: how much of a transfer is staged is read from its blob.

The plaintext payload has the frame of the other stores, all integers
big-endian:

    u16 payload version     1
    u32 records length
    records
    zero padding to a multiple of 64 KiB

and the records are:

    u16 count               at most MAX_TRANSFER_RECORDS
    count x record:
      [16] transfer_id
      [16] identity_id        IdentityId of the local identity
      [16] contact_id         ContactId of the contact record
      u8   direction          0 outgoing, 1 incoming
      u8   state              below
      u8   flags              below
      u8   reason             why it failed or who cancelled it,
                              0 in any other state
      u64  size               declared size in bytes
      u8   name_len           1..255
      name_len bytes name     the filename of the offer
      [32] digest             outgoing: of the staged file; incoming: as
                              verified, zero before
      [16] blob_id            zero when there is no blob
      u64  created            Unix time when the record was made
      u64  deadline           incoming offer: when it expires; a
                              record that can stall: its stall
                              deadline; ended incoming record: when
                              it may go; else 0 (below)

States: outgoing 1 waiting, 2 sending, 3 delivered; incoming 11
offered, 12 accepted, 13 complete, 14 expired; both 20 rejected, 21
cancelled, 22 failed. Flags: bit 0, the outgoing offer is known to have
been read by the peer (ADR 0013, a Pong after it); bit 1, the peer is
not known to have read the decision or the cancellation yet; bit 2, the
incoming file was saved; bit 3, a save of the incoming file began and
did not end; bit 4, the user removed the ended incoming transfer from
the interface, and its record stays only so that a repeated offer finds
it. `reason` says why a transfer failed or who cancelled it, and is 0
in every other state. Failed: 1 storage, 2 the digest did not match, 3
stalled (both directions), 4 the offerer no longer has it (incoming
only). Cancelled: 16 by this side, 17 by the peer, 18 the contact ended
(both directions).

The store fails to validate, as a vault that fails to validate, if:

- the payload version is not 1, the records length does not match, a
  padding byte is not zero, or a record is cut short;
- the count is above `MAX_TRANSFER_RECORDS`, or `size` is above
  `MAX_FILE_SIZE`, the ceiling of the format;
- a `transfer_id`, `identity_id` or `contact_id` is zero, or two records
  have the same (identity, contact, direction, transfer) or the same
  non-zero `blob_id`;
- a state is not one of the direction, a flag is not one of the
  direction and state (bit 0 only outgoing; bit 1 only outgoing
  cancelled and incoming accepted, complete, rejected, expired,
  cancelled and failed; bits 2 and 3 only incoming complete, and not
  both; bit 4 only incoming ended: rejected, expired, cancelled, failed,
  or complete), a reserved bit is set, or `reason` is zero in a failed or
  cancelled state or not zero in any other, or not one of the codes of
  its state;
- `blob_id` is zero in a state that has a blob (outgoing waiting and
  sending; incoming accepted, and complete unless saved or removed) or
  not zero in one that has none, or `digest` is zero where the state
  needs it (outgoing; incoming complete) or not zero where it must be
  (every other incoming state);
- `deadline` is zero where it is needed, or not zero where it is not.
  It holds: for an incoming offer, when it expires; for a record that
  can stall (outgoing waiting and sending, outgoing cancelled with bit
  1, incoming accepted), its last progress plus `TRANSFER_STALL_LIMIT`
  (ADR 0013); for an ended incoming record (rejected, expired, cancelled,
  failed, or complete and saved or removed), when its record may go. It
  is zero for every other record: an ended outgoing one, and a complete
  incoming one neither saved nor removed;
- a name is not a valid filename of `PROTOCOL.md` section 9;
- a `contact_id` names a contact of another local identity.

The writer applies the same checks before it writes, so that what is
written always reads back (`STORAGE.md` section 3.4); in particular a
local file whose name is not a valid filename is offered under the
valid name ADR 0013 makes of it.

When the vault opens and neither `transfers` nor `transfers.new`
exists, or a `transfers.new` alone does not open, an empty store is
made, as for an installation made before Phase 8. Blobs found then are
not named by any record: they are reported, as a missing message store
is, and then removed (below).

### Blobs

A blob is a header and segments. The header is not encrypted.

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `MNLTBLB\0` |
| 8 | 2 | format version, 1 |
| 10 | 2 | reserved, 0 |
| 12 | 16 | blob_id |
| 28 | 4 | segment length L, 64490 in format version 1 |

then, for i = 0 to ceil(size / L) - 1, where `size` is the size of the
record:

| Size | Field |
| --- | --- |
| 24 | nonce, random for every write of the segment |
| L + 16 | XChaCha20-Poly1305(blob_key, nonce, plaintext, ad) |

Every segment holds L plaintext bytes: the last holds the
`size - i * L` remaining bytes of the file followed by zero bytes up to
L, so the length of a blob shows the size of its file only to within a
segment. The associated data is

    ad = header (32 bytes) || u64 size || u64 i || u8 final

with `size` taken from the record, which the store authenticates, and
`final` 1 for the last segment and 0 for the others. Segment i starts at
`32 + i * (40 + L)`. A file of size zero is a header with no segment.

L is a constant of format version 1, equal to the full chunk size of the
frame parameters of protocol version 1 (64490 bytes), so a segment is a
chunk: the sender reads one segment for each FileChunk, and the receiver
writes each chunk as one segment. Any other L is refused before any
arithmetic. If the frame parameters change, so does the format version.

Reading checks the magic, the version, the reserved field, that the
blob identifier is the one the record names and that L is the constant;
then that `size` is at most the ceiling, with every position computed
without overflow; every segment must authenticate under its index, the
size and its final flag, and the padding of the last segment must be
zero. A segment moved within the blob, taken from another blob, cut, or
followed by data where the record says the file ends fails. A blob in
which no segment authenticates yet (a file of size zero, or an incoming
transfer with nothing staged) is never authenticated as a whole: its
header is matched against the record, and its length against the
reserved length. Plaintext segment buffers are held in `Zeroizing`.

The digest of a transfer is the SHA-256 of the `size` bytes of the file,
never of the padding. The bytes a blob holds after n whole segments are
`min(n * L, size)`; that is the offset a FileAccept states.

### Staging, and how much is staged

- A blob is reserved on disk at its full length, `32 + k * (40 + L)` for
  k segments, when it is created, with `FILE_FREE_SPACE_MARGIN` left over.
  The margin is at least what the stores need to be written once more
  (`MAX_MESSAGE_STORE_FILE_LEN`, `MAX_TRANSFER_STORE_FILE_LEN` and
  `MAX_VAULT_FILE_LEN`, with room to spare), so a reservation never makes
  a write of a store fail. On Unix the free space is read with `fstatvfs`
  on the directory, as the blocks available to the user (`f_bavail`
  times `f_frsize`), and the length is allocated with `fallocate` on
  Linux; where that call is refused (some network, FUSE and older ZFS
  file systems) or does not exist, the file is only extended to its
  length after the free-space check. On Windows, which gives no safe
  call to read the free space, the file is extended to its full length
  plus the margin, which allocates its clusters on an ordinary NTFS
  volume and fails if the space is missing, and then shortened to its
  full length. A reservation holds the lock of the disk of the
  installation, which every write of a store (vault, message store,
  transfer store) holds too, so that no write of a store runs while the
  margin is taken. The lock gives mutual exclusion within the process and
  nothing more: a crash in the middle of a reservation or of a write of a
  store leaves what the crash table of `STORAGE.md` section 8 says, as it
  would without the lock. That a write of a store is whole or absent
  after a crash comes from its write order (`STORAGE.md` section 3.5),
  and that a crash leaves at most a blob no record names comes from the
  order of the next sections, not from the lock. A blob for which the space is
  missing is not created. The reservation is best effort: a compressed
  or sparse file, a copy-on-write file system or kept snapshots may need
  new space for a write after all, which the failures below cover.
  Whatever a region holds that this blob never sealed for that position,
  zero bytes or old data, fails to authenticate.
- The sender stages a whole file before the record exists. It takes the
  size of the source when it opens it, refuses a source above
  `MAX_TRANSFER_SIZE`, creates and reserves the blob, writes the header
  and each segment as it reads, hashes the plaintext with SHA-256 as it
  goes, and fails if the source gives fewer or more bytes than that
  size. It syncs the file and the directory, and only then writes the
  record (waiting, with the digest). The source is read once and never
  again: what is offered is what the blob holds.
- The receiver creates and reserves the blob, header written, when the
  user accepts: it syncs the file and the directory, and only then
  writes the record that says accepted and names the blob. A transfer of
  size zero has its complete blob at that point.
- The receiver writes one segment per chunk, each at its computed
  position `32 + n * (40 + L)`. A FileAccept never counts more than the
  last sync of the blob covered: before it sends one that counts new
  segments, and before it verifies the file, it syncs the blob.
- The first time an installation uses an incoming accepted blob after it
  opened, it authenticates its segments in order up to the first that
  fails, syncs the blob, and keeps the number of staged segments in
  memory; everything after the last good segment counts as not written,
  and is written over. A later scan may count again an older segment
  that a write left in place beyond a failure; it is this blob's own
  ciphertext for that position, and the digest check covers what it
  holds. After that it advances that number with each synced segment and
  reads nothing again for a new session. A segment
  written again gets a new random nonce, so a key and nonce never meet
  two plaintexts.
- Outgoing blobs and complete incoming blobs are never scanned at open.
  Each segment is authenticated when it is read, for a chunk, a
  verification or a save, and the first failure stops there.

### Missing and damaged blobs

What a record finds when it uses its blob decides:

- Incoming, accepted: a missing blob, a header that does not match, or
  a length other than the reserved length, counts as nothing staged: a new blob is created, reserved, synced, and
  named by the record before any segment counts; if it cannot be
  reserved, the transfer is held as after a full disk (below). A segment
  that fails to authenticate counts as not written, with every segment
  after it.
- Outgoing, and incoming complete: a missing blob, a header that does
  not match, a length that does not match, or a segment that fails makes
  the transfer failed, with the reason storage. The record is written
  and the user is told. Nothing is written over, and the installation
  opens whatever a blob holds.

These cases are in the crash table of `STORAGE.md` section 8.

### Saving a received file

The front end asks for a reader of a complete incoming transfer. The
reader authenticates and decrypts the blob segment by segment, hashes
what it returns, and, after the last byte, returns an error, never the
end of the file, unless the length and the hash are those of the
record. Before it writes anything, the record gets the flag that a save
began, durably. The front end then:

1. finds out whether the folder takes a hard link, with a probe file of
   no content, created with exclusive create, linked and removed;
2. creates a temporary file with exclusive create in the folder the
   user chose, named `monolith-partial-` followed by 16 random
   hexadecimal digits, a visible name;
3. writes what the reader gives, and stops at its first error;
4. after the reader ended without an error, syncs the file;
5. gives it the final name without replacing anything: a hard link,
   which fails if the name exists, and the temporary name removed; on
   Linux, where the file system takes no hard link (`EPERM`,
   `EOPNOTSUPP`), a rename that refuses an existing name (`renameat2`
   with `RENAME_NOREPLACE`);
6. syncs the folder (on Windows by flushing a handle of the folder,
   with the caveat of `STORAGE.md` section 3.8);
7. only then says that the file is saved.

A failure before step 5 removes the temporary file, clears the flag
that a save began, and leaves the staged blob untouched. A folder that
takes neither a hard link nor, on Linux, the rename of step 5 (refused
with `EINVAL` or `ENOSYS`, as on some network and FUSE file systems) is
refused for saving at step 1, before anything is written, with a message
that says why; on Windows no safe call renames without replacing, so
there a folder without hard links is refused. A failure after step 5
(the folder sync, or removing the temporary name) leaves the complete
final file, says that the save could not be confirmed, keeps the blob
and clears the flag. Nothing is ever written under the final name
directly.

A crash during a save leaves the flag set: at the next start the user is
told that a save of that file was interrupted and that a file named
`monolith-partial-...` may be left in the folder it was going to; the
flag is then cleared. Monolith keeps no record of the folder.

Once the file is saved, the record gets the saved flag in place of the
flag that a save began, durably, and then the blob is removed.

### Removing, and what is left after a crash

- A blob is created, and synced with its directory, before the record
  that names it. The record is written without the blob before the blob
  is removed. A crash between two steps leaves at most a blob no record
  names.
- Removal waits for a reader of the blob (a chunk, a verification, a
  save) to end, or cancels it; it never removes a blob that is being
  read.
- Only once the transfer store has opened and validated, every name of
  the blob pattern that no record names is removed. A record whose local
  identity or contact record no longer exists is removed, and its blob
  after it.
- These removals, the removal of a `transfers.new` that does not
  authenticate, and the writing over of segments after the last good one
  of an incoming accepted blob are the
  exceptions this format adds to the rule of `STORAGE.md` section 3.5
  that nothing which does not authenticate is deleted; that section
  lists them.
- Staged data is removed: for an outgoing transfer, once it is
  delivered, rejected, cancelled or failed; for an incoming transfer,
  once it is cancelled or failed, saved, or removed by the user. A
  complete incoming transfer that was neither saved nor removed is not
  ended: its blob stays, counted in `MAX_STAGED_INCOMING_BYTES`, until the
  user saves or removes it.
- Records of ended incoming transfers stay for `ENDED_TRANSFER_RETENTION`
  and then go; those of ended outgoing ones go when the user removes
  them. Making room follows ADR 0013: never a record of a transfer that
  is not ended, nor one that says the peer is not known to have read a
  decision or a cancellation.

### Failures

- A write of a blob by the receiver that fails because the disk is full
  or the file is held by another process (sharing violation, access
  denied) is not damage: everything after the last synced segment
  counts as not written, the receiver acknowledges nothing more and ends
  the session once.
  It then holds the transfer: the transfer stays accepted, its offers are
  answered with nothing, as an undecided offer is, and it makes no claim
  on the scheduler, until `TRANSFER_HOLD` has passed or the user resumes
  it; the user is told why it waits. The blob keeps its reservation.
- Any other failure of a blob write by the receiver fails the transfer:
  the failed record is written durably first, then the blob is removed.
- A staging by the sender that fails, for any reason, removes its blob
  and is reported to the user; no record was written, so nothing else
  changes.
- A removal that fails leaves a blob no record names; it is not a
  failure of the store. The installation tries the removal again with
  each write of the transfer store and at the next open. A blob is held
  open while its transfer sends or receives, so that another process
  that opens it, such as a scanner, holds it less often.
- A write of the transfer store that fails is handled as a failed write
  of the message store (ADR 0008, S53): the store fails closed, every
  session ends, and no message and no transfer is stored or sent until
  the process starts again; the vault is unaffected.

### Ephemeral installations

An ephemeral installation stages in memory, in the same format, under a
`staging_key` and a `store_key` drawn from `getrandom` when it starts and
never written anywhere. A blob is a list of segment buffers, each
allocated when its segment is written, with fallible allocation; nothing
is allocated from a declared size. The declared sizes of the transfers
staged in memory, both directions, add up to at most
`MAX_EPHEMERAL_STAGED_BYTES`. Nothing is written to disk, as S25
requires, and nothing is left when the process ends. Saving a received
file writes the plaintext to the place the user chose, as in a
persistent installation; that is the user's act, as saving a card is.

### Limits of the protection

- The length of a blob shows the size of its file to within a segment
  from the moment the blob is created; since every blob is reserved at
  its full length, the length alone shows neither how far an incoming
  transfer is nor its direction. The metadata of the file system does,
  to someone who holds the medium: allocated but unwritten extents on
  Linux, the valid data length on NTFS. The birth time of a blob shows
  when a file was picked or accepted, its modification time when data
  last arrived (an outgoing blob stops changing once staged), and that
  of `transfers` when a transfer changed state. Names of blobs are random
  and carry nothing.
- An old copy of the vault, with the passphrase it had, decrypts every
  later blob and transfer store, as `STORAGE.md` section 3.2 says of the
  message store.
- Rolling back the whole profile, or `transfers` alone, to an earlier
  copy cannot be detected: it can bring back a rejected or cancelled
  offer, or a record whose blob was removed, which then fails as above.
- Removing a blob does not reach copies the file system or the medium
  keep (journals, copy-on-write, flash translation): `STORAGE.md` section
  1 says the same of every store.
- Memory is not locked; a plaintext segment and the keys can reach swap
  (`CRYPTOGRAPHY.md` section 8).
- A saved file is plaintext where the user put it, and a crash while it
  is saved can leave its partial plaintext there under the temporary
  name.

## Consequences

- `monolith-storage` gets streaming access to files of the profile
  directory (create with a reservation, write and read at an offset,
  sync, list the blob names) on Unix and Windows, and in the in-memory
  directory, whose crash model gains: a write at an offset that is lost,
  or torn with a prefix of the new bytes and the old bytes after it (a
  stale segment of the same blob among them); unsynced writes made
  durable out of order, a later one surviving an earlier one; reserved
  regions that read as zero bytes; and a reservation and a header that
  are durable only after their sync. Its oracle accepts that a scan
  counts again an older segment of the blob that a later write left in
  place. Also the blob and transfer store formats with test vectors
  written from this ADR alone, and the two derived keys.
- `STORAGE.md` (layout, formats, the exceptions of section 3.5, crash
  table, section 4 on received files), `CRYPTOGRAPHY.md` (keys and their
  lifetimes), `THREAT_MODEL.md` (adversary J, and section N, where a
  declared size now reserves disk space, but only after the user accepted
  and within `MAX_STAGED_INCOMING_BYTES`), `PROTOCOL.md` 13.3 and
  `RESOURCE_LIMITS.md` follow. Section 4 of `STORAGE.md`, which put the
  temporary file in the folder the user chose and the received file in
  plaintext, is replaced.
- An installation that opens a store of a later format version refuses
  it, as for the vault and the message store.

## Alternatives considered

- Transfer records in the message store: it is rewritten whole, holds at
  most 16 MiB and needs a new payload version for new kinds of records;
  a separate small store changes neither.
- One counter nonce per segment under a per-blob key (the STREAM
  construction): safe while each index is written once, but the receiver
  writes the tail again after a crash, possibly with other bytes from a
  dishonest sender; random nonces need no such rule. `aead-stream` would
  also be a new dependency.
- The size in the header and an exact last segment: simpler, but the
  length of the blob would give the size of the file exactly.
- Plaintext staging in a private directory (`PROTOCOL.md` 13.3 of
  version 1): leaves received and sent files in plaintext in the
  profile, against `STORAGE.md`.
- Staging in the folder the user chose (`STORAGE.md` section 4 of
  version 1): plaintext, and the peer's file would reach a user folder
  before the user decided to save it.
- Writing a saved file directly under its final name, or renaming over a
  name that exists: a crash would leave a partial file under the final
  name, or replace a file of the user.
