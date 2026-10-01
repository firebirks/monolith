# Dependencies

Every third-party crate that is compiled into Monolith is recorded here with
the reason it is used and what was checked before it was added. A crate is
added by a change that also updates this file. Candidates that are not in
use yet are in the ADRs.

Facts were checked on 2026-10-01 against crates.io, the crate sources as
downloaded by cargo, `cargo audit` and `cargo deny`.

## 1. Rules

- A dependency is chosen for maintenance, review history, advisories,
  transitive size, exposure of unsafe code to hostile input, and licence.
  The language a cryptographic library is written in decides nothing by
  itself.
- No pre-release versions of cryptographic crates.
- One implementation of each primitive. `deny.toml` forbids two versions of
  the same crate.
- Features are switched off by default and enabled one by one.
- Every version in `Cargo.lock` must build with the minimum supported Rust
  version, 1.85.1. CI checks it.
- Monolith's own crates forbid `unsafe`. Dependencies may contain it; where
  they do, the count below says how much, as a pointer for review and not
  as a verdict. The count is the number of lines under `src/` that contain
  the keyword, without comments and lint attributes.

## 2. Generation of the RustCrypto and dalek crates

Two generations exist side by side:

| | Previous | Current |
| --- | --- | --- |
| `ed25519-dalek` | 2.2.0 (2025-07-09) | 3.0.0 (2026-07-06) |
| `curve25519-dalek` | 4.1.3 (2024-06-18) | 5.0.0 (2026-07-06) |
| `sha2` | 0.10.9 (2025-04-30) | 0.11.0 (2026-03-25) |

Monolith uses the current generation.

It started on the previous one and moved before any session or storage
cryptography was written, so that the major-version step did not have to
be taken with more code depending on it. The move needed no source change:
the calls Monolith makes (`VerifyingKey::from_bytes`, `verify_strict`,
`is_weak`, `to_edwards`, `is_torsion_free`, `SigningKey::from_bytes`,
`Sha256`) are the same in both generations. Every known-answer test, the
strict-verification vector and the committed fuzz seeds are unchanged, so
no signed or wire byte changed.

Reasons:

- The previous generation is at its end. `curve25519-dalek` 4.1.3 is its
  newest release that was not withdrawn (4.2.0 was yanked), and
  `ed25519-dalek` has had no 2.x release since 2.2.0. A fix that lands only
  in the current generation would have forced the move later, under
  pressure.
- The current generation supports Rust 1.85, the project's minimum.
- It is a coherent family: one `curve25519-dalek`, one `sha2`, one
  `digest`.

Risks, recorded so that they are not forgotten:

- The stable releases of `ed25519-dalek` 3 and `curve25519-dalek` 5 are
  three months old. They have had less use than the previous generation.
- No public audit of these major versions is known.
- `snow` 0.10, one candidate for the session layer, depends on the
  previous generation. Choosing it would put a second curve implementation
  and a second `sha2` into the binary, which `deny.toml` forbids. ADR 0002
  has to weigh that; it is not a reason to move back.

Features that stay off unless a decision record asks for them and says
why: `legacy_compatibility` (verification rules that accept encodings
strict verification rejects) and `hazmat` (access to expanded secret keys
and prehashed signing).

## 3. Crates in use

Production dependencies of `monolith-identity` and `monolith-protocol`.

### Direct

| Crate | Version | Licence | MSRV | Used for | Features |
| --- | --- | --- | --- | --- | --- |
| `ed25519-dalek` | 3.0.0, pinned exactly | BSD-3-Clause | 1.85 | identity signatures, key validation | `fast`, `zeroize`; no default features |
| `sha2` | 0.11.0 | MIT OR Apache-2.0 | 1.85 | identity fingerprint | none; no default features |
| `subtle` | 2.6.1 | BSD-3-Clause | not declared | constant-time comparison of invitation capabilities | no default features |
| `unicode-normalization` | 0.1.25 | MIT OR Apache-2.0 | 1.36 | NFC check of display names | no default features |

`ed25519-dalek`

- Maintained by the dalek-cryptography organization. 3.0.0 was released on
  2026-07-06 after a year of pre-releases.
- Enabled: `fast` (precomputed tables in `curve25519-dalek`) and `zeroize`
  (signing keys are wiped when dropped). These are the crate's own
  defaults, listed explicitly.
- Not enabled: `legacy_compatibility`, `hazmat`, `rand_core`, `batch`,
  `digest`, `serde`, `pkcs8`, `pem`, `alloc`. None of them is needed, and
  the first two change what the crate accepts or exposes.
- Review: Quarkslab audited the dalek libraries in 2019. The main scope was
  `curve25519-dalek` and `subtle`; `ed25519-dalek` got a brief look. That
  predates the 2.x and 3.x lines. No later public audit is known.
- Advisories: RUSTSEC-2022-0093 (signing oracle through a mismatched public
  key), fixed in 2.0. Not applicable to 3.0.0.
- Unsafe: none.
- Usage rules in Monolith: verification is always `verify_strict`; keys are
  additionally checked for canonical encoding, small order and a torsion
  component (`PROTOCOL.md` section 10.1).
  Signing keys are built from 32 bytes supplied by the caller; the crate's
  random number feature is not enabled.

`sha2`

- Maintained by RustCrypto. Already in the tree through `ed25519-dalek`,
  which uses SHA-512.
- Not enabled: `alloc`, `oid`, `zeroize`.
- Advisories: RUSTSEC-2021-0100, fixed long before 0.10.
- Unsafe: 50 lines, in the CPU-specific compression back ends.

`subtle`

- Maintained by the dalek-cryptography organization. Already in the tree
  through `ed25519-dalek` and `curve25519-dalek`; adding it as a direct
  dependency adds no code to the build.
- Review: in the main scope of the 2019 Quarkslab audit of the dalek
  libraries.
- Advisories: none.
- Unsafe: 2 uses. One is the volatile read that keeps the optimizer from
  turning the comparison back into an early exit; the other converts a
  constant-time ordering result.
- Used for one thing: `InvitationCapability` equality. It is a direct
  dependency of `monolith-protocol` because a hand-written comparison loop
  has no such barrier against the optimizer.

`unicode-normalization`

- Maintained by the unicode-rs organization. Not a cryptographic crate.
- Advisories: none.
- Unsafe: 5 lines.
- Used for one question only: is this display name already in
  Normalization Form C. A wrong answer cannot corrupt memory or a signed
  structure; at worst a name is accepted or rejected wrongly.

### Transitive

| Crate | Version | Licence | Through | Unsafe lines |
| --- | --- | --- | --- | --- |
| `curve25519-dalek` | 5.0.0 | BSD-3-Clause | `ed25519-dalek` | 33 |
| `curve25519-dalek-derive` | 0.1.1 | MIT OR Apache-2.0 | `curve25519-dalek`, build time | - |
| `ed25519` | 3.0.0 | Apache-2.0 OR MIT | `ed25519-dalek` | 0 |
| `signature` | 3.0.0 | Apache-2.0 OR MIT | `ed25519` | 0 |
| `digest` | 0.11.3 | MIT OR Apache-2.0 | `sha2`, `curve25519-dalek` | 0 |
| `block-buffer` | 0.12.1 | MIT OR Apache-2.0 | `digest` | 21 |
| `crypto-common` | 0.2.2 | MIT OR Apache-2.0 | `digest` | 0 |
| `hybrid-array` | 0.4.15 | MIT OR Apache-2.0 | `digest` | 37 |
| `typenum` | 1.20.1 | MIT OR Apache-2.0 | `hybrid-array` | 0 |
| `zeroize` | 1.9.0 | Apache-2.0 OR MIT | `ed25519-dalek` | 15 |
| `cpufeatures` | 0.3.1 | MIT OR Apache-2.0 | `sha2`, `curve25519-dalek` | 11 |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | several | 0 |
| `tinyvec` | 1.13.3 | Zlib OR Apache-2.0 OR MIT | `unicode-normalization` | 0 |

`curve25519-dalek` is taken as `ed25519-dalek` requires it, with the
features `digest`, `precomputed-tables` and `zeroize`. Its
`legacy_compatibility`, `rand_core`, `group`, `lizard` and `alloc` features
are off. RUSTSEC-2024-0344 (timing variability in scalar subtraction) was
fixed in 4.1.3 and does not apply. Its unsafe code is in the vectorized
back ends. The back end is chosen by the crate for the target; Monolith
sets no back-end flag. `fiat-crypto` 0.3.0 appears in `Cargo.lock` for the
back end of that name, which is selected only by a compiler flag that
Monolith does not set, so it is not compiled.

`hybrid-array` replaces `generic-array` of the previous generation.

Build-time only: `proc-macro2`, `quote`, `syn`, `unicode-ident` (for
`curve25519-dalek-derive`), `rustc_version`, `semver` (build script of
`curve25519-dalek`).

### Development only

| Crate | Version | Licence | MSRV | Used for |
| --- | --- | --- | --- | --- |
| `proptest` | 1.11.0 | MIT OR Apache-2.0 | 1.85 | property tests |

`monolith-protocol` also lists `ed25519-dalek` as a development
dependency. One test needs point arithmetic to build a key with a torsion
component.

`proptest` brings a random number stack (`rand` 0.9 and others) into test
builds. None of it is linked into a release binary. The fuzz targets under
`fuzz/` are a separate cargo project with their own lock file and use
`libfuzzer-sys`.

## 4. Checks

Run on the dependency set above on 2026-10-01:

| Check | Result |
| --- | --- |
| `cargo audit` | no advisory applies |
| `cargo deny check` (advisories, bans, licences, sources) | passes with the repository policy |
| Build and tests with Rust 1.85.1 | pass |
| Highest declared MSRV in the tree | 1.85 (the dalek and RustCrypto crates, `zeroize`, `proptest`) |

The highest declared MSRV equals the project's. A later release of any of
these crates may need a newer compiler; the resolver will then keep the
older version, and the MSRV job in CI fails if it cannot.

## 5. Not yet chosen

- Session layer and its crypto back end: ADR 0002.
- Storage: `argon2`, `chacha20poly1305`, `hkdf`, and the message store:
  ADR 0005.
- Randomness: `getrandom`, when key generation is implemented.
- Onion address checksum: `sha3`, with the Tor backend.
- Runtime, logging, GUI: with the phases that need them.

## 6. Sources

- https://crates.io/crates/ed25519-dalek/versions
- https://crates.io/crates/curve25519-dalek/versions
- https://rustsec.org/advisories/RUSTSEC-2022-0093.html
- https://rustsec.org/advisories/RUSTSEC-2024-0344.html
- https://blog.quarkslab.com/security-audit-of-dalek-libraries.html
- https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/struct.VerifyingKey.html
- https://crates.io/crates/sha2/versions
