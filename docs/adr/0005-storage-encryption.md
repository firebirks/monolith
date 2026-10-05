# ADR 0005: Storage encryption

Status: the vault is decided and implemented in `monolith-storage` (Phase
4, waiting for its final verification); its default KDF parameters are
provisional until measured on the four targets; the message store is
open.
Date: 2026-10-01. Amended 2026-10-05 by Phase 4: the vault implemented,
the payload layout written down (`STORAGE.md` section 3.4), ST2, ST4 and
ST7 decided (`DESIGN_QUESTIONS.md` P4-6).

## Context

Persistent state has to be protected against someone who obtains the disk
but not the passphrase. It has to work on Tails and Whonix without a
desktop keyring. The identity key must never be at risk from an interrupted
write. Identity and contacts are small and change rarely; message history is
large, changes constantly and is optional.

## Decision

### Vault (identity and contacts)

A single file, rewritten whole and replaced atomically, in the format of
`STORAGE.md` section 3:

- Argon2id derives a key-encryption key from the passphrase. The proposed
  default of 256 MiB, 3 iterations and 4 lanes is provisional and has to be
  benchmarked on a desktop, a modest virtual machine, a Whonix-Workstation
  and Tails before it is frozen (`STORAGE.md` section 3.3). The floor is
  RFC 9106's second recommended setting. Parameters are stored in the
  header and bounded on read, so existing vaults stay decryptable when the
  default rises.
- A random 256-bit vault key is wrapped under that key.
- The payload is encrypted with XChaCha20-Poly1305 under a key derived from
  the vault key with HKDF-SHA256. The whole header is associated data.
- Random 192-bit nonces.

This is a container of Monolith's own definition. It was chosen over
existing formats because none fits (STORAGE.md 3.1), and it is kept to the
most ordinary possible use of two standard primitives. It needs review.
Its test vector is a file built from fixed inputs by a writer written from
`STORAGE.md` section 3.2 alone, which the vault must open and whose SHA-256
is pinned; a reader written the same way opens what the vault writes
(`crates/monolith-storage/tests/vault_format.rs`).

As implemented in Phase 4: one vault per persistent installation holds
every local identity (ST7); the directory is locked with an exclusive
`flock`, and its owner and permissions are checked before anything is read
(ST2, ST4), through `rustix` without `unsafe`; a write is a complete new
file, flushed, renamed over the vault and the directory flushed; a
creation is made under another name and linked into place, so a crash
never leaves a vault that is half created. The crash behavior is tested
against a model of what a crash keeps after each step (`MemoryDir`).

A passphrase is mandatory for a persistent vault in version 1.

### Message store

Not decided. Recommended candidate: SQLCipher through `rusqlite`, linked to
the system library on Debian-based targets, opened with a raw key derived
from the vault key. Alternatives and their costs are in `STORAGE.md`
section 5. Phase 4 did not take the decision: its brief was the contact
store, and the message store comes with the outbound queue
(`DESIGN_QUESTIONS.md` section 11.3, ST1).

History is off by default on every platform, so the store is not needed for
a working messenger.

## Why not one mechanism for both

Putting identity and contacts into SQLCipher would bring a C library and
OpenSSL's libcrypto into the path that protects the identity key, for data
that is a few kilobytes and rewritten rarely. Putting history into a
whole-file container would rewrite the entire history on every message.
The two kinds of data have different needs.

## Dependencies

| Crate | Current | Notes |
| --- | --- | --- |
| `argon2` | 0.6.0 (2026-08) | RustCrypto. No advisories. |
| `chacha20poly1305` | 0.11.0 (2026-06) | RustCrypto. The AEAD crates were audited by NCC Group in 2019; that covers old code. No advisories. |
| `hkdf` | 0.13.0 | RustCrypto. No advisories. |
| `zeroize` | 1.9.0 | |
| `rusqlite` | 0.40.2 | Needs Rust 1.88, above the current minimum. An older release or a higher minimum would be required. |
| SQLCipher | 4.6.1 in Debian 13; 4.19.0 upstream | BSD-style licence with an attribution requirement. |

The generation is decided: Monolith uses the current one
(`DEPENDENCIES.md` section 2), so storage uses `chacha20poly1305` 0.11 and
the matching `argon2` and `hkdf`. `snow` 0.10 depends on the previous
generation (`chacha20poly1305` 0.10, `sha2` 0.10); that conflict is ADR
0002's to resolve and does not change the choice for storage.

## Consequences

- The vault can be unlocked anywhere with the passphrase alone.
- Changing the passphrase does not re-key the message store.
- Unlock costs time and memory once per start. How much is measured on
  the development machine only (`STORAGE.md` section 3.3).
- Rollback of the vault file by someone with write access is not
  detectable.
- Monolith owns a small file format and its migration path.

## Alternatives considered

- `age` passphrase files: beta crate, scrypt, no subkey. See STORAGE.md 3.1.
- Keyring only: not available on every target, and Monolith has to work
  without one; may be added as an extra unlock method.
- No encryption, rely on disk encryption: Tails' Persistent Storage is
  encrypted, but an unlocked Persistent Storage would then expose the
  identity key to any process and any backup of the files.
- `sled`: still a beta with a rewrite in progress. `redb`: stable and pure
  Rust, but offers no encryption; listed as a message store option.

## Open questions

STORAGE.md section 9: ST1, ST3, ST5 and ST6 are open; ST2, ST4 and ST7 are
decided.

## Sources

Accessed 2026-10-01.

- https://www.rfc-editor.org/rfc/rfc9106
- https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html
- https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
- https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305
- https://docs.rs/age/0.12.1
- https://www.zetetic.net/sqlcipher/license/
- https://packages.debian.org/trixie/libsqlcipher1
