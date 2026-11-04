# Storage

Status: the vault (section 3) is implemented in Phase 4, in
`monolith-storage`, with the record layout of section 3.4 and the crash
tests of section 8; Phase 5 adds the durable identifiers of payload
version 2. Its KDF defaults are provisional until the benchmark of section
3.3 has been run on every target. The message store (section 5) is
implemented in Phase 5 (`adr/0008-message-store.md`). The transfer store and
the encrypted blobs of file transfers (section 5.1 to 5.6) are implemented in
Phase 8 (`adr/0014-transfer-staging.md`).

## 1. What is stored, and where

Four kinds of state, kept apart:

| State | Contents | Where |
| --- | --- | --- |
| Identity and contacts | identity key, transport key, Onion Service key, card epoch, invitations, contacts with their pinned cards (identity key, transport key, endpoints, epoch), block and declined lists, verification marks, settings that are not secret | the vault |
| Messages | the outbound queue, delivered and received messages, the identifiers of deleted received messages | the message store (section 5); in memory in ephemeral mode |
| Transfers | the record of every file transfer (direction, state, name, size, digest), and the staged data of the files being sent or received | the transfer store and the blobs (section 5.1); in memory in ephemeral mode |
| Runtime | sessions, session keys, isolation tokens, pending requests and offers, UI queue | memory |

Since Phase 5 a persistent installation always has a message store: a
message is acknowledged only once it is durably held there
(`adr/0007-delivery-semantics.md`), and a queue survives a restart. What a
conversation holds is kept until the user deletes it. `HistoryPolicy` in
`monolith-storage` stays a type for a later choice of keeping less; it
switches nothing in version 1.

Modes:

- Ephemeral: nothing is written. Identity, endpoint and contacts live in
  memory and are gone when the process exits. Default on Tails.
- Persistent: the vault exists in a directory the user chose or confirmed.
  Default on other Linux systems and on Whonix.

In ephemeral mode the messages and the transfers live in memory with
everything else and are gone when the process exits. In persistent mode the
messages are in the message store and the transfers in the transfer store
and the blobs, each encrypted under a key derived from the vault key
(sections 5 and 5.1).

Data directory: `$XDG_DATA_HOME/monolith`, falling back to
`~/.local/share/monolith`. On Tails there is no default outside the
Persistent Storage; see `PLATFORM_TAILS.md` section 5. The desktop client
uses `/home/amnesia/Persistent/monolith` there while the Persistent
Storage is unlocked, and otherwise offers only a session that keeps
nothing. On Windows the data directory is `%LOCALAPPDATA%\Monolith\Profile`
(section 3.8).

    monolith/            0700
      vault              0600   encrypted identity and contacts
      vault.new          0600   only during a write
      lock               0600   single-instance lock
      messages           0600   message store (section 5)
      messages.new       0600   only during a write
      transfers          0600   transfer store (section 5.1)
      transfers.new      0600   only during a write
      transfer-<32 hex>  0600   staged data of one transfer (section 5.2)

Monolith writes no log file. Logs go to standard error or the journal and
contain nothing sensitive (S18).

### 1.1 Several local identities

An installation may hold several local identities (`ARCHITECTURE.md`
section 1.1). Every stored record belongs to one of them:

    vault
      +-- identity A   keys, onion key, cards and capabilities, contacts,
      |                block and declined lists, labels and settings
      +-- identity B   the same, separately
    message store
      +-- (A, record)  queue, messages, identifiers of deleted messages
      +-- (B, record)  the same, separately
    transfer store
      +-- (A, record)  transfers with the contact, their staged data
      +-- (B, record)  the same, separately

- Records of contacts, blocked, declined and former contacts, verification
  marks, aliases and pending requests in both directions are keyed by the
  local identity and the remote identity. Queued messages, history and the
  identifiers of deleted messages are keyed by the durable identifier of
  the local identity and of the contact record (`IdentityId`, `ContactId`,
  section 3.4). A conversation is never identified by the remote identity
  alone, and a contact deleted and made again, or an identity deleted and
  restored from its keys, starts with none of it. No record of one
  identity applies to another; a block list for all identities, if ever
  offered, is a separate record and an explicit choice.
- In version 1 one vault, under one passphrase, may hold several
  identities. A vault per identity is possible later and is not planned.
- Deleting a local identity removes its keys, its Onion Service key, its
  cards and capabilities, its contact state, its queued messages and its
  history, and nothing of any other identity. As in section 6, it does not
  reach older copies, backups or freed disk blocks.
- A backup or export (section 7) may cover one identity or all of them.
  Each identity keeps its identity key, transport key, Onion Service key,
  capabilities and contacts together; keys are never moved from one
  identity to another.
- An identity whose identity, transport or Onion Service key equals one
  held by another local identity is refused at creation, import or
  restore (S39).

Ephemeral and persistent identities are not mixed in one process in
version 1: `StateMode` belongs to the installation (ST7, decided in Phase
4). A persistent installation keeps every identity it holds in its vault;
an ephemeral one writes nothing.

## 2. Threat addressed

An attacker who obtains the storage medium, or a copy of the data directory,
but not the passphrase (adversary J in the threat model). They must not
learn the identity key, the transport key, the Onion Service key, who the contacts are, or any
message, and must not be able to modify the state without detection.

Not addressed:

- An attacker with access while the vault is unlocked. Keys are in memory.
- Rollback. An attacker who can write to the directory can replace the vault
  with an older valid copy. Nothing on the same disk can detect that.
- Traces on the medium. Old ciphertext may survive in freed blocks on SSDs
  and copy-on-write or journaling filesystems. Monolith does not promise
  secure deletion and offers no "wipe" function.
- The size of the vault and the times it was written are visible.

## 3. Vault

### 3.1 Why a format of its own

The requirement is a small file, rewritten whole, protected by a
passphrase, with a key that can also protect a second store. Alternatives:

- `age` (passphrase mode): a reviewed format, but the Rust crate declares
  itself beta before 1.0, has no public audit, uses scrypt where Argon2id
  is preferred, and offers no way to derive a stable subkey for a second
  store.
- SQLCipher for everything: page-level encryption with good tooling, but it
  brings a C library and OpenSSL into the process that holds the identity
  key. See section 5.
- Operating system keyrings: not present or not unlocked on every target.
  Monolith has to work without them.

The vault is therefore a minimal container built in the most conventional
way from two standard primitives: a memory-hard KDF and one AEAD. There is
no novel construction in it. Every byte of the header is authenticated.

### 3.2 Layout, version 1

All integers big-endian.

| Offset | Size | Field | Notes |
| --- | --- | --- | --- |
| 0 | 8 | magic | `MNLTVLT` followed by 0x00 |
| 8 | 2 | format_version | 1 |
| 10 | 1 | kdf | 1 = Argon2id, version 0x13 |
| 11 | 1 | reserved | 0 |
| 12 | 4 | kdf_memory_kib | |
| 16 | 4 | kdf_iterations | |
| 20 | 4 | kdf_parallelism | |
| 24 | 16 | kdf_salt | random |
| 40 | 24 | wrap_nonce | random |
| 64 | 48 | wrapped_vault_key | 32-byte key and 16-byte tag |
| 112 | 8 | generation | incremented on every write; see 3.5 |
| 120 | 24 | payload_nonce | random, new on every write |
| 144 | 4 | payload_len | length of the field that follows, tag included |
| 148 | payload_len | payload | ciphertext and 16-byte tag |

The file is exactly 148 + `payload_len` bytes long. A file with any other
length is rejected.

Keys:

    kek         = Argon2id(passphrase, kdf_salt, memory, iterations, parallelism), 32 bytes
    vault_key   = XChaCha20-Poly1305-Open(kek, wrap_nonce, wrapped_vault_key, ad = bytes 0..64)
    payload_key = HKDF-SHA256(vault_key, info = "MONOLITH-VAULT-PAYLOAD-V1"), 32 bytes
    plaintext   = XChaCha20-Poly1305-Open(payload_key, payload_nonce, payload, ad = bytes 0..148)

- HKDF is used as RFC 5869 defines it, extract then expand, with an empty
  salt and the `info` string shown. Every derived key is 32 bytes.
- `vault_key` is 32 random bytes generated when the vault is created. The
  passphrase protects it; it protects everything else. Changing the
  passphrase re-wraps the same key with a new salt and nonce, so the
  message store key (section 5) stays valid.
- Because the vault key survives a passphrase change, an old copy of the
  vault file together with the passphrase it had at the time still yields
  the key to every later vault and to the message store. A passphrase that
  may have been exposed therefore calls for the separate rekey operation:
  a new vault key, the vault re-encrypted under it, and the message store
  re-keyed.
- The associated data of the payload is the entire header, including the
  KDF parameters and the generation. Changing any header byte makes opening
  fail.
- Nonces are 192-bit random values. With random nonces of that size,
  collisions are not a concern at any number of writes a vault will see.
- The plaintext is padded with zero bytes to a multiple of 64 KiB, so the
  file size reveals the number of contacts only in coarse steps. For almost
  all users the vault is a single 64 KiB block.

### 3.3 KDF parameters

Proposed default, provisional: Argon2id, 256 MiB, 3 iterations, 4 lanes,
16-byte salt. These numbers come from reasoning, not from measurement, and
are not frozen.

RFC 9106's second recommended setting (64 MiB, 3 iterations, 4 lanes) is the
floor and stays the floor. The proposed default uses more memory because
the vault is unlocked once per start on a desktop.

What is fixed regardless of the final numbers:

- The vault header stores the parameters it was created with (section 3.2),
  so a vault stays decryptable when the default changes later.
- Defaults only move up. A vault created with weaker parameters keeps
  working and is re-wrapped with the current default the next time its
  passphrase is changed. (Phase 4: `Vault::change_passphrase` takes the
  parameters to wrap with; the installation offers no passphrase change
  yet, so the upgrade comes with the interface.)
- Monolith runs one key derivation at a time. A second unlock attempt waits
  for the first, so concurrent attempts cannot multiply the memory use.
  (Phase 4: the lock of the vault directory refuses a second open of the
  same vault while one holds it, in this process or another; the
  derivations of different vaults in one process are not serialized yet.)

#### Benchmark required before the default is frozen

To be run with the real implementation, in Phase 4, on at least:

| Target | Notes |
| --- | --- |
| Ordinary Linux desktop | reference |
| Modest virtual machine | 2 cores, 2 GB RAM |
| Whonix-Workstation | default memory allocation of the current release |
| Tails in a virtual machine | 2 GB RAM, the documented minimum |

Measured for each candidate setting: time to unlock (median and worst of
ten runs), peak resident memory, free memory during the run, whether the
kernel's out-of-memory handling was triggered, responsiveness of the
desktop during the run, and the same with several unlock attempts issued at
once.

A setting is acceptable if it is expensive enough to matter against
password guessing, unlocks in a time a user will tolerate at every start,
never triggers out-of-memory handling on the smallest target, and does not
stall the rest of the system. Where these conflict, the cost to an attacker
is not traded away for convenience: the answer to a slow unlock on a small
machine is a documented choice for that machine, not a lower default for
everyone.

Results so far, with the implementation of Phase 4
(`crates/monolith-storage/tests/kdf_benchmark.rs`, built with
optimization, ten unlocks per setting), 2026-10-05:

| Machine | Setting | Median | Worst | Peak resident |
| --- | --- | --- | --- | --- |
| Desktop, Intel Core i5-11400, 12 threads, 32 GB, Debian 12 | 64 MiB, 3, 4 (floor) | 118 ms | 159 ms | 65 MiB |
| same | 256 MiB, 3, 4 (proposed default) | 464 ms | 522 ms | 257 MiB |
| same | 256 MiB, 3, 2 | 390 ms | 456 ms | 257 MiB |
| same, in a container limited to 2 CPUs and 2 GB | 64 MiB, 3, 4 | 92 ms | 95 ms | 65 MiB |
| same | 256 MiB, 3, 4 | 391 ms | 444 ms | 257 MiB |
| same, limited to 1 CPU and 1 GB | 64 MiB, 3, 4 | 97 ms | 112 ms | 65 MiB |
| same | 256 MiB, 3, 4 | 382 ms | 410 ms | 257 MiB |

The `argon2` crate is used without its `parallel` feature, so the lanes
are computed one after another on one thread: the number of lanes changes
little, and a limit on processors does not slow an unlock down. A
container limit does not model a slower processor or the memory pressure
of a small virtual machine; the four targets above remain to be measured,
and the default stays provisional until they are (ST6). On the desktop
the proposed default costs half a second and 257 MiB once per start, and
no run came near the memory limit of the container.

Accepted when reading a header (`limits.rs`): memory from 64 MiB to 1 GiB,
iterations from 3 to 16, parallelism from 1 to 8. Values outside are
rejected before Argon2 runs. Without an upper bound, a modified header could
make unlock allocate arbitrary memory; without a lower bound, a bug or a
modified file could weaken a vault silently.

The passphrase is normalized to NFC, and the normalized form is limited to
1024 bytes of UTF-8. A persistent
vault always has a passphrase in version 1. Unlocking through a desktop
keyring may be added later as an additional way to obtain the vault key; it
will never be the only one.

### 3.4 Payload

A versioned sequence of typed records in the same fixed-layout style as the
wire protocol, each set of them belonging to one local identity (section
1.1). Version 1 (`monolith-storage`, `record.rs`): big-endian integers,
a presence byte (0 or 1) before every optional field, and a 16-bit length
before every field of variable size.

    payload     u16 version (1 or 2), u32 records length, records, zero padding
    records     u16 identity count, identity*
    identity    u8 tag (1), identity id [16] (version 2),
                identity seed [32], transport secret [32],
                presence + Onion Service secret [64], card epoch u64,
                endpoint [32], presence + rotation, request mode u8,
                presence + local label, u8 invitation count, invitation*,
                u16 contact count, contact*, u16 blocked count, key [32]*,
                u16 declined count, key [32]* (oldest first)
    rotation    new transport secret [32], successor epoch u64, switched u8
    invitation  capability [16], presence + local label
    contact     contact id [16] (version 2),
                kind u8 (1 requested, 2 accepted), active card,
                presence + authorized successor card, presence + pending
                card, pending was imported u8, presence + retired key [32],
                presence + capability [16], card confirmed for dialing,
                verified u8, presence + alias, successor announced u8,
                successor promoted u8
    card        u16 length, the card as on the wire (section 11.1 of
                PROTOCOL.md)
    label       u16 length, display-name text (section 9 of PROTOCOL.md)

Version 2 (Phase 5) adds the durable identifiers of
`adr/0008-message-store.md`: 16 random bytes per identity and per contact
record, drawn when it is made and never changed or reused; zero bytes and
duplicates are refused. A version 1 payload is still read; its records get
identifiers when the vault is opened, and the vault is written as version
2, after everything in it was checked and before anything can refer to
them. A payload is written back in the version it was read in, so it has
one encoding.

Secrets are the bytes the keys were made from; the key types themselves
cannot be exported. The local card is not stored: it is signed again
from the seed, the transport key, the epoch and the endpoint when the
vault is opened.

Rules that are fixed now:

- Records are built from validated typed values only. Display names and
  profile text are stored as length-prefixed byte strings that already
  passed protocol validation. Nothing peer-supplied is ever interpreted as
  structure (S28).
- A reader that meets a record type or payload version it does not know
  refuses to open the vault. It does not skip and continue.
- The payload is bounded: the file is at most `MAX_VAULT_FILE_LEN`, and
  every count in it is checked against the ceiling of the format before
  anything is allocated for it: the limit of `limits.rs` it was made with,
  which a later build may lower without making a vault above it
  unreadable (`DESIGN_QUESTIONS.md` P7-5).
- Everything read is validated again: cards are decoded and their
  signatures verified, credentials are rebuilt with `Credentials::restore`,
  which refuses any state no transition could have produced, labels pass
  the display-name rules, no remote identity has two records, secrets of
  zero bytes are refused, and a rotation must move the epoch forward with
  another key: another public key, since X25519 clamps the scalar and two
  secrets can be one key. Opening also refuses two identities that share a
  key (S39). Padding after the records must be zero, so a payload has one
  valid encoding.
- What is written is what can be read back: creating, restoring and
  rotating an identity apply the same rules to the secrets before they are
  accepted (`record::is_usable_secret`), so no accepted identity makes the
  vault fail to open.

### 3.5 Writing

Every step below happens while holding the single-instance lock.

Every change writes the whole vault:

1. Serialize, pad, encrypt with a fresh `payload_nonce` and
   `generation + 1`.
2. Create `vault.new` with exclusive-create and mode 0600. Write. `fsync`.
3. Rename `vault.new` to `vault`.
4. `fsync` the directory.

Creating a vault differs in step 3. `vault.new` is hard-linked to `vault`,
which fails if `vault` exists, and then `vault.new` is removed. A vault is
therefore never created over an existing one, and `vault` never exists in a
partly written state.

Where rename is atomic, `vault` is always either the old complete state or
the new complete state. The identity key is never modified in place; it is
carried along in each complete new file.

At startup, after the lock is taken and the passphrase has been entered:

- Only `vault` exists: open it.
- Both exist: if `vault` authenticates, use it and remove `vault.new`,
  which is a write that did not commit. If `vault` does not authenticate
  and `vault.new` does, stop and tell the user; `vault.new` replaces
  `vault` only with the user's confirmation.
- Only `vault.new` exists: if it authenticates, it is a creation that was
  interrupted after the file was complete. Link it to `vault`, remove
  `vault.new`, and tell the user. If it does not authenticate, leave it
  alone and report it.

Monolith never deletes or overwrites a file that it has not been able to
authenticate, with one exception: a `vault.new` next to a `vault` that
authenticates. The message store, which follows these cases with its own
names, has a second one: a `messages.new` alone that does not open is
removed before an empty store is made (`docs/adr/0008-message-store.md`,
Writing; since Phase 7, P7-20, said here). The transfer store follows the
same cases with the names `transfers` and `transfers.new` and the same
second exception. Phase 8 adds the others, all listed in section 5.4:
the blobs no record names, the records of contacts the vault no longer
holds, and the segments written over after the last good one of an
incoming accepted blob.

`generation` lets a person or a recovery tool tell which of two valid files
is newer. It does not detect rollback: an older valid file is still a valid
file.

### 3.6 Failure

- Wrong passphrase and modified file are indistinguishable by design; both
  fail authentication. The message says so. A message store that does not
  authenticate once the vault did is changed or damaged, and is said to be
  damaged, not a wrong passphrase (P7-18).
- A vault that does not open is left untouched. Monolith does not offer to
  "reset" it; that would destroy the identity. The user can move the file
  away by hand and start a new identity.
- A newer `format_version` is refused. An older one is migrated by writing
  the new format through the normal atomic replace. There is no downgrade.
- Only one process may use a data directory. A second one finds the lock
  held and exits.

### 3.7 Permissions

The directory is 0700 and files are 0600, set at creation, not after. At
open, Monolith refuses a data directory or vault that is group or world
accessible, is a symbolic link, or is not owned by the current user.

### 3.8 Windows

Since Phase 6 the vault directory also runs on Windows
(`crates/monolith-storage/src/disk_windows.rs`, P6-13). The files, their
formats, the order of writes and the recovery of section 3.5 are those of
Unix; what differs is how the operations are made:

- The directory is held open for as long as the vault is, without the
  sharing that would let another process rename or delete it (see the
  limitations below for what this does not cover).
- Nothing that is a reparse point (a symbolic link or a junction) is
  followed: each file is opened as itself and refused if it is one, as
  Unix refuses a symbolic link.
- The single-instance lock (`lock`) is a file opened with no sharing at
  all. A second process cannot open it, and Windows releases it when the
  process ends, however it ends.
- A file is created only if it does not exist and flushed with
  `FlushFileBuffers` before it is renamed; the directory's names are
  flushed through a handle of the directory opened for writing. A rename
  is that of the standard library, which replaces its target. As the
  source of Rust 1.95 shows, it calls `MoveFileExW` with
  `MOVEFILE_REPLACE_EXISTING` and without `MOVEFILE_WRITE_THROUGH`; only
  if that is refused with `ERROR_ACCESS_DENIED` does it rename through
  `SetFileInformationByHandle` with `FileRenameInfoEx` and POSIX
  semantics. A rename refused with `ERROR_SHARING_VIOLATION` or
  `ERROR_ACCESS_DENIED`, which is what another process that holds the
  file or its target without the sharing that allows it (a scanner, an
  indexer) causes, is tried again after pauses of 10 ms to 800 ms, about
  1.6 s in all, and only then fails the store (since Phase 7,
  `DESIGN_QUESTIONS.md` P7-10). A second name is a hard link.

What Unix checks and Windows does not: who owns the directory and who else
may read it. Windows keeps that in access control lists, which the
standard library cannot read, and Monolith uses no code of its own outside
it. The default data directory is `%LOCALAPPDATA%\Monolith\Profile`, inside
the user's profile, whose list gives access to the user, SYSTEM and the
administrators; a directory elsewhere has the list its parent gives it.
This is a documented limitation of the Windows build, not a check skipped
on Unix.

Two more limitations of the Windows build:

- Microsoft does not document that `FlushFileBuffers` on a handle of a
  directory makes a rename in that directory durable, as `fsync` of the
  directory does on Unix. It is the nearest call Windows offers, not a
  documented guarantee that a rename survives a crash.
- Only the last component of the directory is held open. Every operation
  names its file by the path of the directory joined with the file name,
  and Windows resolves that path again on each operation.

## 4. Temporary files, and saved files

Staged data is not a temporary file: a transfer is held in a blob of the
profile directory, encrypted (section 5.2), until the user saves it or
removes it. The files this section covers are the ones Monolith writes
outside the profile directory, in a folder the user chose, when the user
saves a received file (section 5.5): a probe file of no content and a
temporary file, both named `monolith-partial-` followed by 16 random
hexadecimal digits, a visible name. The final name never replaces an
existing file. A failed or cancelled save removes the temporary file; a
crash can leave one behind, and the interface tells the user at the next
start which save was interrupted. Monolith does not search folders for
partial files, and keeps no record of the folder.

Received files are plaintext where the user saves them. That is what
saving a file means.

## 5. Message store and transfer store

Decided in Phase 5: `adr/0008-message-store.md`, which closes ST1 for
version 1. In short:

- One file, `messages`, beside the vault and under its lock, a container
  of the vault's kind without a passphrase: XChaCha20-Poly1305 under

      store_key = HKDF-SHA256(vault_key, info = "MONOLITH-MESSAGE-STORE-V1")

  with the 48-byte header as associated data, the plaintext padded to
  64 KiB, at most 16 MiB (`MAX_MESSAGE_STORE_FILE_LEN`). It is rewritten
  whole and replaced atomically like the vault (section 3.5, with the
  names `messages` and `messages.new`), by a write job of its own, with
  generations of its own.
- Its payload holds, per local identity, the conversations of its contact
  records: queued, delivered and received messages in the order they were
  stored, with the local time, and the identifiers of received messages
  the user deleted. Reading checks every count before allocating, the
  order, the identifiers and every text; nothing that fails is repaired.
- At opening it is matched against the vault: the messages of an identity
  the vault does not hold are dropped, a conversation of a record the vault
  does not hold is kept as ended, and one of a record of another identity
  is refused. Where no store is in place an empty one is made; for a
  vault that holds identities (a copy of the vault alone, which is a
  backup of its identities) that is reported. A vault of version 1 beside
  a store with messages is refused.
- SQLCipher through `rusqlite` was not taken: `rusqlite` needs Rust 1.88,
  above the minimum supported version, and SQLCipher brings C code and
  OpenSSL into the process. A store for long histories is later work.

### 5.1 The transfer store

Decided in Phase 8: `adr/0014-transfer-staging.md`. One file, `transfers`
(and `transfers.new` while it is written), beside the vault and under its
lock: the records of every transfer of the installation, in both
directions. It is built as the message store is, with its own magic
(`MNLTXFR` followed by 0x00) and its own key,

    store_key = HKDF-SHA256(vault_key, info = "MONOLITH-TRANSFER-STORE-V1")

a 48-byte header authenticated as associated data, a random nonce for
every write, a generation of its own, and a payload padded with zero bytes
to a multiple of 64 KiB (`TRANSFER_STORE_PADDING_BLOCK_LEN`). The file is
at most `MAX_TRANSFER_STORE_FILE_LEN` (1 MiB) and holds at most
`MAX_TRANSFER_RECORDS` records. A newer format version is refused.

A record holds:

| Field | Meaning |
| --- | --- |
| transfer id | 16 bytes the offerer drew |
| identity id, contact id | the local identity and the contact record (section 3.4); a transfer belongs to both |
| direction | outgoing or incoming; the same transfer id from two contact records, or in two directions, is two transfers |
| state | outgoing: waiting, sending, delivered; incoming: offered, accepted, complete, expired; both: rejected, cancelled, failed |
| flags | the outgoing offer is known to have been read by the peer; the peer is not known to have read the decision or the cancellation yet; the incoming file was saved; a save began and did not end; the user removed the ended incoming transfer from the interface |
| reason | why it failed (storage, the digest did not match, stalled, the offerer no longer has it) or who cancelled it (this side, the peer, the contact ended); zero in every other state |
| size, name | the declared size in bytes and the filename of the offer (1 to 255 bytes, a valid filename of `PROTOCOL.md` section 9) |
| digest | SHA-256 of the file: outgoing, of the staged file; incoming, as verified, zero before |
| blob id | the blob that holds the staged data; zero when there is none |
| created, deadline | Unix times; the deadline is when an incoming offer expires, the stall deadline of a record that can stall, or when an ended incoming record may go; zero otherwise |

A record has a blob in these states only: outgoing waiting and sending,
incoming accepted, and incoming complete unless the file was saved or
removed.

Reading and writing apply the same checks, so that what is written always
reads back (section 3.4). The store is refused, as a vault that fails to
validate is, if: the payload version or the records length is wrong, a
padding byte is not zero, or a record is cut short; the count or a size is
above the ceiling of the format (`MAX_FILE_SIZE`); an identifier is zero,
or two records share their (identity, contact, direction, transfer) or a
non-zero blob id; a state does not belong to its direction, a flag or a
reason does not belong to its state, or a reserved bit is set; a blob id is
zero where the state has a blob, or not zero where it has none; a digest
is zero where the state needs one, or not zero where it must be zero; a
deadline is zero where the state needs one, or not zero where it does not;
a name is not a valid filename; or a contact record belongs to another
local identity.

#### Writing

The store is rewritten whole and replaced atomically, with the steps of
section 3.5 and the names `transfers.new` and `transfers`, by one write job
of its own. Every write of a store (vault, message store, transfer store)
and every reservation of a blob takes the one lock of the disk of the
installation, which keeps a reservation from using the space the next write
of a store needs. The lock is mutual exclusion within the process and
nothing more: what a crash leaves comes from the order of the writes
(section 8), not from the lock.

A change is used only once it is durable: a message is written, a command
reported done, an event raised and a state shown only after the write that
holds it ended. The store is written when a transfer changes state, and
for the stall deadline of a transfer at most once per
`TRANSFER_PROGRESS_RECORD_INTERVAL`; never for each chunk. How much of a
transfer is staged is read from its blob, not from the store.

A write of the store that fails is treated as a failed write of the
message store: the store fails closed, every session ends, and no message
and no transfer is stored or sent until the process starts again. The
vault is unaffected.

#### What the interface is shown

The interface reads the transfers as the transfer store holds them
durably. A change in memory that is not yet written is never shown, so a
crash cannot take back what the user saw: a transfer shown as accepted, as
complete, as saved or as cancelled is one a restart finds in that state.
Beside the records the runtime keeps in memory what they do not hold, and
rebuilds it after a restart: how much of a transfer is known to be staged
or acknowledged, a transfer held after a full disk, and the sessions in a
row without an offer.

#### Opening

When the vault has opened, the transfer store is opened as the message
store is (section 3.5) and matched against the vault:

- If neither `transfers` nor `transfers.new` exists, or a `transfers.new`
  alone does not open, an empty store is made, as for an installation made
  before Phase 8. The blobs found then are named by no record: that is
  reported, as a missing message store is, and the blobs are removed.
- A record whose local identity or contact record the vault no longer
  holds goes, and its blob after it. This is what a crash leaves of the
  deletion of a contact or an identity.
- A record of a contact record that belongs to another local identity is
  refused as corrupt.
- Only after the store has opened and validated, every file whose name is a
  blob name (section 5.2) that no record names is removed.

### 5.2 Blobs

The staged data of one transfer is a file of the profile directory named
`transfer-` followed by exactly 32 lower-case hexadecimal digits, the blob
identifier: 16 random bytes, never zero, drawn when the blob is created and
drawn again if a file of that name exists or a record names it. A name is
a blob only if it matches that pattern exactly; `transfers` and
`transfers.new` are not blobs. A name is never made from anything a peer
sent.

A blob is a 32-byte header, which is not encrypted, and segments:

| Offset | Size | Field |
| --- | --- | --- |
| 0 | 8 | magic `MNLTBLB` followed by 0x00 |
| 8 | 2 | format version, 1 |
| 10 | 2 | reserved, 0 |
| 12 | 16 | blob identifier |
| 28 | 4 | segment length L, 64490 in format version 1 |

then, for each of the ceil(size / L) segments, 24 bytes of nonce and
L + 16 bytes of sealed data (`CRYPTOGRAPHY.md` section 9.1). Every segment
holds L bytes of plaintext: the last holds the remaining bytes of the file
followed by zero bytes. The length of a blob therefore shows the size of
its file only to within a segment. A file of size zero is a header with no
segment. L is the full chunk size of protocol version 1, so a segment is a
chunk; any other L is refused before any arithmetic.

Reading checks the magic, the version, the reserved field, that the blob
identifier is the one the record names, and that L is the constant. Every
segment must authenticate under its index, the size and its final flag, and
the padding of the last must be zero. Plaintext segment buffers are cleared
when dropped.

Every file that transfers write in the profile directory is created with
exclusive create, mode 0600 on Unix, without following a link or a
reparse point, in the directory the installation holds locked, and is
opened again the same way with the owner and mode checks of the other
files.

### 5.3 Staging, and what a blob holds

- A blob is reserved on disk at its full length when it is created, with
  `FILE_FREE_SPACE_MARGIN` left over, under the lock of the disk. The
  margin is at least what the stores need to be written once more, so a
  reservation never makes the write of a store fail. On Unix the free space
  is read with `fstatvfs` and the length allocated with `fallocate` on
  Linux; where that call is refused or does not exist, the file is only
  extended to its length after the check. On Windows, which gives no safe
  call to read the free space, the file is extended to its full length plus
  the margin, which allocates its clusters on an ordinary NTFS volume and
  fails if the space is missing, and then shortened to its full length. A
  blob for which the space is missing is not created. The reservation is
  best effort: a compressed or sparse file, a copy-on-write file system or
  kept snapshots may need new space for a write after all.
- The sender stages the whole file before the record exists. It takes the
  size of the source when it opens it, refuses a source above
  `MAX_TRANSFER_SIZE`, creates and reserves the blob, writes each segment as
  it reads and hashes the plaintext, and fails if the source gives fewer or
  more bytes than its size. It syncs the file and the directory, and only
  then writes the record (waiting, with the digest). The source is read once
  and never again: what is offered is what the blob holds.
- The receiver creates and reserves the blob, header written, when the user
  accepts. It syncs the file and the directory, and only then writes the
  record that says accepted and names the blob.
- The receiver writes one segment per chunk at its computed position. It
  acknowledges no more than the last sync of the blob covered: it syncs the
  blob before it sends an acceptance that counts new segments, and before
  it verifies the file.
- The first time an installation uses an incoming accepted blob after it
  opened, it authenticates the segments in order up to the first that
  fails, syncs the blob and keeps the number of staged segments in memory.
  Everything after the last good segment counts as not written and is
  written over. A segment written again gets a new random nonce.
- Outgoing blobs and complete incoming blobs are not scanned at open. Each
  segment is authenticated when it is read, for a chunk, a verification or
  a save, and the first failure stops there.
- An ephemeral installation stages in memory, in the same format, under
  keys drawn from the system when it starts and written nowhere. A blob is
  a list of segment buffers, each allocated when its segment is written,
  with fallible allocation. The declared sizes of the transfers staged in
  memory add up to at most `MAX_EPHEMERAL_STAGED_BYTES`. Nothing is written
  to disk and nothing is left when the process ends.

What a record finds when it uses its blob decides:

- Incoming accepted: a missing blob, a header that does not match, or a
  length other than the reserved one counts as nothing staged. A new blob
  is created, reserved, synced and named by the record before any segment
  counts. A segment that fails to authenticate counts as not written, with
  every segment after it.
- Outgoing, and incoming complete: a missing blob, a header or a length
  that does not match, or a segment that fails, makes the transfer failed
  with the reason storage. The record is written and the user is told.
  Nothing is written over.

A write of a blob by the receiver that fails because the disk is full or
the file is held by another process (sharing violation, access denied) is
not damage. Everything after the last synced segment counts as not
written, the receiver acknowledges nothing more and ends the session once.
It holds the transfer: it stays accepted and makes no claim on the
scheduler until `TRANSFER_HOLD` has passed or the user resumes it, and the
user is told why it waits. Any other failure of a blob write by the
receiver fails the transfer: the failed record is written durably first,
then the blob is removed. A staging by the sender that fails removes its
blob and is reported; no record was written, so nothing else changes.

### 5.4 Removing, and what a crash leaves

- A blob is created, and synced with its directory, before the record that
  names it. The record is written without the blob before the blob is
  removed. A crash between two steps leaves at most a blob no record names.
- Removal waits for a reader of the blob (a chunk, a verification, a save)
  to end, or cancels it; it never removes a blob that is being read.
- Staged data is removed: for an outgoing transfer, once it is delivered,
  rejected, cancelled or failed; for an incoming transfer, once it is
  cancelled or failed, saved, or removed by the user. A complete incoming
  transfer that was neither saved nor removed is not ended: its blob stays,
  counted in `MAX_STAGED_INCOMING_BYTES`, until the user saves or removes
  it.
- A removal that fails leaves a blob no record names. It is not a failure
  of the store: the removal is tried again with each write of the transfer
  store and at the next open.
- Records of ended incoming transfers (rejected, expired, cancelled, failed,
  and complete and saved or removed) stay for `ENDED_TRANSFER_RETENTION`, 14
  days, and then go, so that an offer repeated meanwhile finds its record.
  Removing a complete transfer from the interface removes its blob and hides
  its record, which stays for that time. Records of ended outgoing
  transfers go when the user removes them. To make room the store never
  removes the record of a transfer that is not ended, nor one that says the
  peer is not known to have read a decision or a cancellation
  (`adr/0013-file-transfer.md`).

The rule of section 3.5, that nothing which does not authenticate is
deleted, has these exceptions in Phase 8, besides a `vault.new` next to a
vault that authenticates and a `messages.new` alone that does not open: a
`transfers.new` alone that does not open, the blobs no record names, the
records of contacts the vault no longer holds, and the segments written
over after the last good one of an incoming accepted blob. No blob is
deleted because it fails to authenticate: a blob that fails makes its
transfer fail, and its record decides when the blob goes.

### 5.5 Saving a received file

The runtime saves the file, not the front end, so that every front end
saves it the same way (`monolith_storage::export`, `save_new`). The user
chooses a folder and a name; the name a peer sent is at most a suggestion.
The place must be one of this computer: on Windows a share, a link or a
junction on the way, or the name of a device or a stream is refused before
anything is opened. The place is checked first: an absolute path, a folder
that exists, no file of that name.

The runtime asks for a reader of the complete transfer. The reader
authenticates and decrypts the blob segment by segment and hashes what it
returns. After the last byte it returns an error, never the end of the
file, unless the length and the SHA-256 are those of the record. Before
anything is written, the record gets the flag that a save began, durably.
Then:

1. Whether the folder takes a hard link is found out with a probe file of no
   content, created only if no file has its name, linked and removed.
2. A temporary file is created with exclusive create in the folder, named
   `monolith-partial-` and 16 random hexadecimal digits.
3. What the reader gives is written to it, up to the first error.
4. After the reader ended without an error, the file is synced.
5. It is given the final name without replacing anything: a hard link, which
   fails if the name exists, and the temporary name removed. On Linux, where
   the file system takes no hard link (`EPERM`, `EOPNOTSUPP`), a rename that
   refuses an existing name (`renameat2` with `RENAME_NOREPLACE`).
6. The folder is synced (on Windows by flushing a handle of the folder, with
   the caveat of section 3.8).
7. Only then is the file reported saved.

A failure before step 5 removes the temporary file, clears the flag that a
save began and leaves the blob untouched. A folder that takes neither a
hard link nor, on Linux, the rename of step 5 is refused at step 1 before
anything is written, with a message that says why; on Windows no safe call
renames without replacing, so a folder without hard links is refused there.
A failure after step 5 (the folder sync, or removing the temporary name)
leaves the complete file under its final name, reports that the save could
not be confirmed, keeps the blob and clears the flag. Nothing is ever
written under the final name directly, and nothing is replaced.

Once the file is saved, the record gets the saved flag in place of the flag
that a save began, durably, and then the blob is removed. A crash during a
save leaves the flag set: at the next start the user is told that the save
of that file was interrupted and that a file named `monolith-partial-...`
may be left in the folder it was going to, and the flag is cleared.

### 5.6 Limits of the protection

- The length of a blob shows the size of its file to within a segment from
  the moment it is created. Every blob is reserved at its full length, so
  the length alone shows neither how far an incoming transfer is nor its
  direction. The metadata of the file system does, to someone who holds
  the medium: allocated but unwritten extents on Linux, the valid data
  length on NTFS. The birth time of a blob shows when a file was picked or
  accepted, its modification time when data last arrived, and that of
  `transfers` when a transfer changed state. Names of blobs are random.
- An old copy of the vault, with the passphrase it had, decrypts every later
  blob and transfer store, as section 3.2 says of the message store. A
  change of passphrase keeps the staged blobs readable; a rekeying of the
  vault, which is not implemented, would have to discard them.
- Rolling back the whole profile, or `transfers` alone, to an earlier copy
  cannot be detected. It can bring back a rejected or cancelled offer, or a
  record whose blob was removed, which then fails as above.
- Removing a blob does not reach copies the file system or the medium keep
  (journals, copy-on-write, flash translation).
- Memory is not locked; a plaintext segment and the keys can reach swap
  (`CRYPTOGRAPHY.md` section 8).
- A saved file is plaintext where the user put it, and a crash while it is
  saved can leave its partial plaintext there under the temporary name.

## 6. Deleting

"Delete contact" asks separately about:

- the contact itself: pinned identity, endpoint, verification mark;
- queued messages to that contact;
- stored history with that contact;
- the transfers with that contact and their staged files (section 5.1);
  a record that outlives its contact is removed, with its blob, when the
  installation next opens;
- whether the identity stays on the block list.

Deleting a contact never makes room for "the same person with a new key".
A later card or request with the same display name and a different identity
key is a different, unknown identity and is shown as such.

Deletion removes data from the current vault or store. It does not remove
it from older copies, backups or freed disk blocks.

## 7. Backups

The vault file is a complete, encrypted backup of every identity it holds:
for each, its keys, its Onion Service key, its contacts and its endpoint
epoch. Copying it while Monolith
is not running is a valid backup. Message history is a separate file and is
included only if the user copies it too. So are the transfer store and the
blobs: a vault copied alone holds no transfer, and where a backup of the
vault meets blobs no record names, they are removed (section 5.1).

There is no plaintext export of secrets. A command that prints or exports a
private key does not exist in version 1.

Hazards the interface must state:

- A restored backup may hold an endpoint epoch lower than one issued after
  the backup was made (PROTOCOL.md open question P3).
- Two running copies of the same identity publish the same Onion Service
  and take connections away from each other.
- An old backup can be opened with the passphrase it had when it was made.

## 8. Crash safety

Operations that must survive interruption at any point, and what a restart
must find:

| Operation | After restart |
| --- | --- |
| Identity creation | either no vault or a complete one; never a partial identity |
| Onion Service key stored | the key Tor returned is in the vault before the contact card is shown |
| Contact acceptance | the contact exists completely or not at all |
| Endpoint update | pinned epoch never lower than before |
| Queue update | no message lost that was reported queued; no message reported delivered without an acknowledgement; no received message acknowledged before it is durable |
| File completion | final file complete, or absent |
| Staging a file to send | no record, and at most a blob no record names, or a record with its whole blob; never a record whose blob was not synced |
| Accepting a file to receive | the offer, or the accepted record with a reserved blob; at most a blob no record names |
| Receiving a chunk | the segments synced before the crash count; the rest is written again, with new nonces |
| Verifying a received file | complete with its digest, or still accepted; the record is durable before the peer is told |
| Removing staged data | the record is written before the blob goes; at most a blob no record names, removed at the next open |
| Saving a received file | the final file complete, or absent; at most a file named `monolith-partial-...` in the folder, and the flag that a save began, which the next start reports and clears |
| Deleting a contact or an identity | records of the transfers go with it at the next open at the latest, and their blobs |

Each has a test that kills the write at every step (T-CRASH): the vault's
own tests stop a write and a creation at each step of `MemoryDir`
(creating the file, writing each half, flushing it, renaming or linking,
flushing the directory), under four outcomes of a crash (names that were
not flushed kept or lost, contents that were not flushed lost or torn), and
`tests/store.rs` in the core does the same for contact creation, card
update, announced successor, promotion, block, deletion, invitation
creation and revocation, the start of a rotation and identity creation. A
new process finds the state before or the state after, never a lower
epoch and never a retired key active again. For the message store,
`messages.rs` in `monolith-storage` stops a write and a creation at each
step, and `tests/messages.rs` in the core crashes the installation at
every step of queueing a message and at each boundary of delivery: before
a message is durable at the sender or at the receiver, after it is
durable and before it is acknowledged, after the acknowledgement and
before the delivery is durable. For transfers, the storage tests stop the
write of the transfer store and of a blob at each step, with the outcomes
of a crash above plus a write at an offset that is lost or torn, writes
made durable out of order, and reserved regions that read as zero bytes
(`MemoryDir`). File completion is the save of section 5.5.

What is written is used only once it is durable (`DESIGN_QUESTIONS.md`
P4-4). The write is a job of its own on the blocking pool, which ends the
same way whatever becomes of the task that started it: a cancelled wait
loses neither the vault nor the lock of its directory. It writes one cut
through every identity, its keys, its rotation and the progress contacts
made in that rotation, its settings, its active set and its contact
store, so a vault never holds an operation half done, more entries than
a bound, or the progress of one rotation stored with another. A step of a rotation reaches peers only
once it is in the vault, so a crash never leaves a contact with a key the
vault does not hold. A new data directory is flushed into its parent
before the vault in it is reported created.

## 9. Open items

ST1. Message store choice. Decided in Phase 5 for version 1
     (`adr/0008-message-store.md`); a store for long histories stays open.

ST2. Locking mechanism. Decided in Phase 4: an exclusive, non-blocking
     `flock` on the `lock` file through `rustix`, which the operating
     system releases when the process ends.

ST3. Whether to keep one previous vault generation for recovery from
     logical corruption. It would keep deleted data around and enable
     rollback by accident. Currently not kept.

ST4. Whether the ownership and permission checks in 3.7 can be done without
     `unsafe` or a new dependency. Decided in Phase 4: with `rustix`
     (`fstat`, `getuid`), without `unsafe` in Monolith; every file is
     opened relative to the directory's descriptor with `O_NOFOLLOW`.

ST5. Passphrase strength policy. Currently a non-empty passphrase and a
     warning for short ones.

ST6. Default Argon2id parameters. Provisional until the benchmark of
     section 3.3 has been run on the four targets; Phase 4 measured the
     development machine only.

ST7. With several local identities (section 1.1): whether `StateMode` and
     `HistoryPolicy` are chosen per identity or for the installation, and
     the record layout that scopes every record to its identity. Decided
     in Phase 4: per installation; the layout is section 3.4.

## 10. Sources

Accessed 2026-10-01.

- Argon2, RFC 9106: https://www.rfc-editor.org/rfc/rfc9106
- OWASP Password Storage Cheat Sheet:
  https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html
- XChaCha20-Poly1305:
  https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
- HKDF, RFC 5869: https://www.rfc-editor.org/rfc/rfc5869
- age crate: https://docs.rs/age/0.12.1
- SQLCipher licence: https://www.zetetic.net/sqlcipher/license/
- SQLCipher in Debian 13: https://packages.debian.org/trixie/libsqlcipher1
- rusqlite: https://github.com/rusqlite/rusqlite
