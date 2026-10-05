# Storage

Status: the vault (section 3) is implemented in Phase 4, in
`monolith-storage`, with the record layout of section 3.4 and the crash
tests of section 8. Its KDF defaults are provisional until the benchmark of
section 3.3 has been run on every target. The message store (section 5) is
an open decision.

## 1. What is stored, and where

Three kinds of state, kept apart:

| State | Contents | Where |
| --- | --- | --- |
| Identity and contacts | identity key, transport key, Onion Service key, card epoch, invitations, contacts with their pinned cards (identity key, transport key, endpoints, epoch), block and declined lists, verification marks, settings that are not secret | the vault |
| Messages | chat history, outbound queue, per-contact duplicate windows | the message store, if enabled |
| Runtime | sessions, session keys, isolation tokens, pending requests and offers, UI queue, partial downloads | memory; partial downloads in temporary files |

Keeping an identity does not imply keeping conversations. The two are
separate switches (`StateMode`, `HistoryPolicy` in `monolith-storage`).

Modes:

- Ephemeral: nothing is written. Identity, endpoint and contacts live in
  memory and are gone when the process exits. Default on Tails.
- Persistent: the vault exists in a directory the user chose or confirmed.
  Default on other Linux systems and on Whonix.

Message history is off by default on every platform, including ordinary
Linux and Whonix. Turning it on is an explicit choice and requires a
persistent vault. With history off, queued unsent messages are held in
memory only; the interface says so when the user quits with messages still
queued.

Data directory: `$XDG_DATA_HOME/monolith`, falling back to
`~/.local/share/monolith`. On Tails there is no default; see
`PLATFORM_TAILS.md` section 5.

    monolith/            0700
      vault              0600   encrypted identity and contacts
      vault.new          0600   only during a write
      lock               0600   single-instance lock
      messages.db        0600   message store, if enabled

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
      +-- (A, peer)    queue, history, duplicate window
      +-- (B, peer)    the same, separately

- Records of contacts, blocked, declined and former contacts, verification
  marks, aliases, pending requests in both directions, queued messages,
  history and duplicate windows are keyed by the local identity and the
  remote identity. A conversation is never identified by the remote
  identity alone. No record of one identity applies to another; a block
  list for all identities, if ever offered, is a separate record and an
  explicit choice.
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

    payload     u16 version (1), u32 records length, records, zero padding
    records     u16 identity count, identity*
    identity    u8 tag (1), identity seed [32], transport secret [32],
                presence + Onion Service secret [64], card epoch u64,
                endpoint [32], presence + rotation, request mode u8,
                presence + local label, u8 invitation count, invitation*,
                u16 contact count, contact*, u16 blocked count, key [32]*,
                u16 declined count, key [32]* (oldest first)
    rotation    new transport secret [32], successor epoch u64, switched u8
    invitation  capability [16], presence + local label
    contact     kind u8 (1 requested, 2 accepted), active card,
                presence + authorized successor card, presence + pending
                card, pending was imported u8, presence + retired key [32],
                presence + capability [16], card confirmed for dialing,
                verified u8, presence + alias, successor announced u8,
                successor promoted u8
    card        u16 length, the card as on the wire (section 11.1 of
                PROTOCOL.md)
    label       u16 length, display-name text (section 9 of PROTOCOL.md)

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
  every count in it is checked against the limits in `limits.rs` before
  anything is allocated for it.
- Everything read is validated again: cards are decoded and their
  signatures verified, credentials are rebuilt with `Credentials::restore`,
  which refuses any state no transition could have produced, labels pass
  the display-name rules, no remote identity has two records, secrets of
  zero bytes are refused, and a rotation must move the epoch forward with
  another key. Opening also refuses two identities that share a key
  (S39). Padding after the records must be zero, so a payload has one
  valid encoding.

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
authenticates.

`generation` lets a person or a recovery tool tell which of two valid files
is newer. It does not detect rollback: an older valid file is still a valid
file.

### 3.6 Failure

- Wrong passphrase and modified file are indistinguishable by design; both
  fail authentication. The message says so.
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

## 4. Temporary files

Incoming transfers are written to a file with a generated name, created
with exclusive-create and mode 0600 in the destination directory the user
chose, so that the final step is a rename on the same filesystem. The
final name never replaces an existing file. An aborted or failed transfer
deletes its partial file; a crash can leave one behind, and Monolith removes
partial files it recognizes at the next start.

Received files are plaintext on disk. That is what saving a file means.

## 5. Message store (open decision)

Needed only when the user turns history on. It holds history, the outbound
queue and duplicate windows.

Candidates:

| Option | For | Against |
| --- | --- | --- |
| SQLCipher through `rusqlite`, keyed with a raw key derived from the vault key | Mature, transactional, random access, hides table structure, per-page authentication so a modified file is rejected before SQLite parses it. In Debian 13 as `libsqlcipher1`. | C code and OpenSSL's libcrypto in the process. Debian's version (4.6.1) lags upstream. Licence requires attribution in the application. Its own cipher choices. |
| Plain SQLite with each value encrypted by Monolith | No OpenSSL. Monolith's primitives. | Row counts, sizes and write patterns are visible. SQLite parses an unauthenticated file. |
| `redb` with each value encrypted by Monolith | Pure Rust, stable file format. | Same visibility as above. |
| No message store in version 1 | Nothing to get wrong. | No history and no queue across restarts. |

Recommendation: SQLCipher, linked against the system library on Debian-based
targets, opened with

    store_key = HKDF-SHA256(vault_key, info = "MONOLITH-MESSAGE-STORE-V1")

passed as a raw key so that SQLCipher's own passphrase KDF is not used. All
statements are parameterized. The decision is not final; it is taken with
the outbound queue and the message store, which follow Phase 4 (ST1,
`DESIGN_QUESTIONS.md` section 11.3), with the dependency review in ADR
0005. Because history is off by default, the choice does not block earlier
phases.

## 6. Deleting

"Delete contact" asks separately about:

- the contact itself: pinned identity, endpoint, verification mark;
- queued messages to that contact;
- stored history with that contact;
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
included only if the user copies it too.

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
| Queue update | no message lost that was acknowledged to the user as queued, when the store is enabled |
| File completion | final file complete, or absent |

Each has a test that kills the write at every step (T-CRASH): the vault's
own tests stop a write and a creation at each step of `MemoryDir`
(creating the file, writing each half, flushing it, renaming or linking,
flushing the directory), under four outcomes of a crash (names that were
not flushed kept or lost, contents that were not flushed lost or torn), and
`tests/store.rs` in the core does the same for contact creation, card
update, announced successor, promotion, block, deletion, invitation
creation and revocation, the start of a rotation and identity creation. A
new process finds the state before or the state after, never a lower
epoch and never a retired key active again. Queue update and file
completion come with the message store and file transfer.

What is written is used only once it is durable (`DESIGN_QUESTIONS.md`
P4-4). The write is a job of its own on the blocking pool, which ends the
same way whatever becomes of the task that started it: a cancelled wait
loses neither the vault nor the lock of its directory. It writes one cut
through every contact store, so a vault never holds an operation half
done or more entries than a bound. A step of a rotation reaches peers only
once it is in the vault, so a crash never leaves a contact with a key the
vault does not hold. A new data directory is flushed into its parent
before the vault in it is reported created.

## 9. Open items

ST1. Message store choice (section 5).

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
