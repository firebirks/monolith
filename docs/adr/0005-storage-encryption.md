# ADR 0005: Storage encryption

Status: proposed for the vault; open for the message store
Date: 2026-10-01

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

- Argon2id derives a key-encryption key from the passphrase. Default
  256 MiB, 3 iterations, 4 lanes; floor at RFC 9106's second recommended
  setting; parameters stored in the header and bounded on read.
- A random 256-bit vault key is wrapped under that key.
- The payload is encrypted with XChaCha20-Poly1305 under a key derived from
  the vault key with HKDF-SHA256. The whole header is associated data.
- Random 192-bit nonces.

This is a container of Monolith's own definition. It was chosen over
existing formats because none fits (STORAGE.md 3.1), and it is kept to the
most ordinary possible use of two standard primitives. It needs review, and
test vectors are part of Phase 4.

A passphrase is mandatory for a persistent vault in version 1.

### Message store

Not decided. Recommended candidate: SQLCipher through `rusqlite`, linked to
the system library on Debian-based targets, opened with a raw key derived
from the vault key. Alternatives and their costs are in `STORAGE.md`
section 5. The decision is taken at the start of Phase 4.

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
| `hkdf` | RustCrypto | |
| `zeroize` | 1.9.0 | |
| `rusqlite` | 0.40.2 | Needs Rust 1.88, above the current minimum. An older release or a higher minimum would be required. |
| SQLCipher | 4.6.1 in Debian 13; 4.19.0 upstream | BSD-style licence with an attribution requirement. |

`snow` 0.10 depends on the previous generation of these crates
(`chacha20poly1305` 0.10, `sha2` 0.10). Using the same generation for
storage avoids two versions of the same cipher in one binary. Decide the
generation once, for all crypto dependencies, at the start of Phase 1,
which is the first phase to need one of them.

## Consequences

- The vault can be unlocked anywhere with the passphrase alone.
- Changing the passphrase does not re-key the message store.
- Unlock costs about a second and 256 MiB once per start.
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

STORAGE.md section 9 (ST1 to ST5).

## Sources

Accessed 2026-10-01.

- https://www.rfc-editor.org/rfc/rfc9106
- https://cheatsheetseries.owasp.org/cheatsheets/Password_Storage_Cheat_Sheet.html
- https://datatracker.ietf.org/doc/html/draft-irtf-cfrg-xchacha-03
- https://github.com/RustCrypto/AEADs/tree/master/chacha20poly1305
- https://docs.rs/age/0.12.1
- https://www.zetetic.net/sqlcipher/license/
- https://packages.debian.org/trixie/libsqlcipher1
