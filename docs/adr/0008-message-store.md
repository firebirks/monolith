# ADR 0008: Message store

Status: decided for Phase 5; closes ST1 (`STORAGE.md` section 9) for
version 1.
Date: 2026-10-11.

## Context

ADR 0007 needs a durable outbound queue, a durable inbox and a durable
record of the identifiers already taken in, each bound to one local
identity and one contact record. ADR 0005 left the message store open
and recommended SQLCipher through `rusqlite`. Since then:

- `rusqlite` needs Rust 1.88, above the minimum supported version
  1.85.1, which is the compiler of Debian 13 and of the Tails and Whonix
  releases Monolith targets. Raising the minimum is a reviewed change of
  its own (`ARCHITECTURE.md` section 10).
- SQLCipher brings C code and OpenSSL's libcrypto into the process that
  holds the identity key.

Phase 4 records a contact by remote identity and numbers its records in
memory only, so a deleted and re-added contact cannot be told from the
old one after a restart. A queue bound to "the contact X" would pass to a
new contact X after a restart. The same holds for a local identity that
is deleted and restored from its keys.

## Decision

### Durable identifiers in the vault

Vault payload version 2 (`STORAGE.md` section 3.4) adds two fields:

- every local identity has an `IdentityId`, 16 random bytes drawn when it
  is created or restored;
- every requested or accepted contact record has a `ContactId`, 16 random
  bytes drawn when the record is made (an import that creates it, an
  accepted request).

Neither ever changes for its record, and neither is reused: a record
deleted or blocked and made again for the same remote identity gets a new
`ContactId`; an identity restored from its keys gets a new `IdentityId`.
A version 1 payload is still read; its records get new identifiers when
the vault is opened, and the vault is written as version 2 before
anything can refer to them. Identifiers of zero bytes, and two records
with one identifier, are refused.

### The store

A second file in the data directory, `messages`, beside the vault and
under its lock. It is a container of the same kind as the vault, without
a passphrase of its own:

| Offset | Size | Field | Notes |
| --- | --- | --- | --- |
| 0 | 8 | magic | `MNLTMSG` followed by 0x00 |
| 8 | 2 | format_version | 1 |
| 10 | 2 | reserved | 0 |
| 12 | 8 | generation | incremented on every write |
| 20 | 24 | nonce | random, new on every write |
| 44 | 4 | payload_len | length of the field that follows, tag included |
| 48 | payload_len | payload | ciphertext and 16-byte tag |

    store_key = HKDF-SHA256(vault_key, info = "MONOLITH-MESSAGE-STORE-V1"), 32 bytes
    plaintext = XChaCha20-Poly1305-Open(store_key, nonce, payload, ad = bytes 0..48)

The plaintext is padded with zero bytes to a multiple of 64 KiB. The file
is at most `MAX_MESSAGE_STORE_FILE_LEN` (16 MiB) and exactly 48 +
`payload_len` bytes long. The key never leaves `monolith-storage`; a
passphrase change keeps it, as `STORAGE.md` section 3.2 says.

Payload, version 1, in the style of the vault records:

    payload       u16 version (1), u32 records length, records, zero padding
    records       u8 identity count, identity*
    identity      identity id [16], u16 conversation count, conversation*
    conversation  contact id [16], peer identity key [32], u64 next sequence,
                  u16 forgotten count, message id [16]*,
                  u32 entry count, entry*
    entry         u64 sequence, u8 kind, message id [16], u64 time,
                  u16 length, text
    kind          1 outbound queued, 2 outbound delivered, 3 inbound

- A conversation belongs to one contact record (its `ContactId`) of one
  local identity (its `IdentityId`). The remote identity key is kept so
  that a conversation can still be shown after its contact is gone.
- `sequence` orders the entries of a conversation as they were stored;
  `time` is the local clock in seconds, for display only.
- `forgotten` holds the identifiers of received messages the user deleted
  locally, at most `MESSAGE_DEDUP_WINDOW`, newest kept, so that a late
  retry of one is acknowledged and not shown again.
- Reading checks everything before it is used: the version, zero padding,
  every count against the ceiling of the format before anything is
  allocated (the limit it was made with; since Phase 7, P7-5), non-zero
  and unique identifiers, entries in increasing sequence below the next
  sequence, known kinds, identifiers unique per direction in a
  conversation, every text against the rules of `ChatText`, the queue
  ceilings. A store that fails any check, or does not authenticate, is not
  opened, and the installation is not opened with it. Nothing is
  repaired by guessing.

### Ownership against the vault

When the installation is opened, the store is matched against the vault:

- the section of an identity that is not in the vault is dropped and the
  store written again: it is what a crash left of a deletion;
- a conversation whose `ContactId` is no contact record of its identity
  is kept, ended: its contact was deleted or blocked. Its queued messages
  are kept and are never sent again;
- a conversation whose `ContactId` is a contact record of another remote
  identity, or of another local identity, is refused as corrupt.

A message is reported queued only once the vault holds its contact record
(its `ContactId` is durable there) and the store holds the message.

### Writing

As the vault (`STORAGE.md` section 3.5) with the names `messages` and
`messages.new`: a new complete file, flushed, renamed over the old one,
the directory flushed. The store is created, empty, with the installation,
by a write under the other name linked into place. Recovery at open
follows section 3.5 with these names: an unauthenticated file is never
removed, except a `messages.new` beside a `messages` that authenticates.

Where no store is in place, an empty one is made, and a `messages.new`
without a `messages`, what an interrupted creation left, is removed
first unless it is a complete store. A vault of payload version 1 never
had a store, and a vault without identities has nothing one could hold.
A vault with identities is a backup of them on its own (Phase 4, T-MI-9),
so it opens without its store too, with an empty one; that the messages
from before are not there is reported (`Installation::message_store_missing`,
`Messenger::message_store_missing`), never passed over. For a vault of
version 1 the store is made, and read, before the vault is written as
version 2. A vault of version 1 beside a store that holds messages (a
vault restored from before the upgrade) is refused: its identities would
get new identifiers, and matching would drop every message.

The store has its own generations and its own write job, the job of the
vault (`monolith-core` `persist`) with another writer: a change is applied
in memory under the lock of the store, stamped, and used only once it is
durable. A failed write fails the store closed: nothing more is taken in
or acknowledged and no message is queued until the process starts again.
The vault and its contacts are not affected.

In an ephemeral installation the store is the same structure in memory
and nothing is written.

### Limits

`RESOURCE_LIMITS.md` section 5: the queue per contact
(`MAX_QUEUED_MESSAGES_PER_CONTACT`) and per local identity
(`MAX_QUEUED_MESSAGES_PER_IDENTITY`), the conversations per identity
(`MAX_CONVERSATIONS_PER_IDENTITY`), the bytes one conversation may hold
(`MAX_CONVERSATION_STORE_LEN`) and the whole store
(`MAX_MESSAGE_STORE_FILE_LEN`). A send above a limit fails and stores
nothing. A received message above a limit is not stored and not
acknowledged; the sender keeps it. Nothing accepted is dropped to make
room. The limits bound what is added; what is read and written is
bounded by the ceilings of the format, the limits it was made with, so
that a store above a limit a later build lowered still opens and grows
no further (`DESIGN_QUESTIONS.md` P7-5).

## Consequences

- No new dependency: the same primitives and the same crash model as the
  vault, tested the same way (`MemoryDir`).
- Every change rewrites the whole file, up to 16 MiB. Writes are
  coalesced by the job, and the bound keeps the cost bounded. A long
  history needs the user to delete conversations; a store that suits
  large histories is later work (ST1 revisited, with the minimum Rust
  version).
- The size of the file and the times it is written are visible, in
  steps of 64 KiB.
- Rollback of the file by someone who can write to the directory is not
  detected, as for the vault. A rolled back store can make a delivered
  message queued again; the receiver drops the retry by identifier.
- Deletion removes data from the current file only; it does not reach
  older copies or freed blocks (`STORAGE.md` section 6).
- `STORAGE.md` section 1: the message store is no longer optional for a
  persistent installation. Acknowledged acceptance needs it. History is
  kept until the user deletes it; an automatic "history off" mode is
  not part of version 1.

## Alternatives considered

- SQLCipher through `rusqlite`: needs Rust 1.88 and C code with OpenSSL;
  see above.
- `redb` or SQLite with values encrypted by Monolith: row counts, sizes
  and access patterns visible; a parser over an unauthenticated file; a
  new dependency.
- An append-only log of encrypted records: cheaper writes, but torn
  appends, compaction and truncation have to be told from tampering. A
  candidate for a later store.
- Growing the vault: the brief rules it out, and contacts would be
  rewritten with every message.
