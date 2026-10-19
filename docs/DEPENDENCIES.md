# Dependencies

Every third-party crate that is compiled into Monolith is recorded here with
the reason it is used and what was checked before it was added. A crate is
added by a change that also updates this file. Candidates that are not in
use yet are in the ADRs.

Facts were checked on 2026-10-01 against crates.io, the crate sources as
downloaded by cargo, `cargo audit` and `cargo deny`. The crates of the
desktop client (Phase 6) were checked the same way on 2026-10-14.

## 1. Rules

- A dependency is chosen for maintenance, review history, advisories,
  transitive size, exposure of unsafe code to hostile input, and licence.
  The language a cryptographic library is written in decides nothing by
  itself.
- No pre-release versions of cryptographic crates.
- One implementation of each primitive. `deny.toml` forbids two versions of
  the same crate in the product. A crate that is in the tree only through
  a development dependency is not counted; section 4 lists the one case.
  The window stack of the desktop client brings second versions of crates
  that are not cryptographic; `deny.toml` accepts each of them by exact
  version, and section 4 lists them.
- An exception to `deny.toml` (an ignored advisory, a banned crate allowed
  under one parent, a second version, a licence) is written there with its
  reason and recorded here. An ignored advisory also carries a review date
  and is ignored for `cargo audit` in `.cargo/audit.toml` as well.
- Features are switched off by default and enabled one by one.
- `deny.toml` bans `native-tls` and `openssl-sys`. This is dependency
  control and says nothing about the quality of OpenSSL's cryptography.
  `native-tls` stays banned because it selects a different TLS
  implementation on each host platform, and Monolith must behave the same
  wherever it runs. `openssl-sys` is banned by default so that it cannot
  enter through a transitive dependency without a decision; ADR 0002 may
  reconsider that ban if the transport review finds a strong technical
  reason. Neither ban is removed without such a record.
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
| `x25519-dalek` | 2.0.1 | 3.0.0 (2026-07-06) |
| `sha2` | 0.10.9 (2025-04-30) | 0.11.0 (2026-03-25) |
| `chacha20poly1305` | 0.10.1 | 0.11.0 (2026-06-28) |

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

- The stable releases of `ed25519-dalek` 3, `curve25519-dalek` 5,
  `x25519-dalek` 3 and `chacha20poly1305` 0.11 are three months old. They
  have had less use than the previous generation.
- No public audit of these major versions is known.
- The crypto resolver that comes with `snow` 0.10 depends on the previous
  generation. Monolith does not use it: the session crate supplies the
  primitives to `snow` from the current generation, for the reasons in
  ADR 0002, F-R1. Avoiding a second copy of these crates was not the
  reason, and it is a consequence.

Features that stay off unless a decision record asks for them and says
why: `legacy_compatibility` (verification rules that accept encodings
strict verification rejects) and `hazmat` (access to expanded secret keys
and prehashed signing).

## 3. Crates in use

Production dependencies of the library crates and of the `monolith`
binary. Phase 3 added `tokio` and `hmac` for the Tor adapter; Phase 4 added
`argon2`, `hkdf`, `rustix` and `unicode-normalization` for the vault.
Phase 6 added the desktop client; its crates are listed under "Desktop
client" below, and none of them is a dependency of the library crates of
the core or of the `monolith` binary.

### Direct

| Crate | Version | Licence | MSRV | Used for | Features |
| --- | --- | --- | --- | --- | --- |
| `ed25519-dalek` | 3.0.0, pinned exactly | BSD-3-Clause | 1.85 | identity signatures, key validation | `fast`, `zeroize`; no default features |
| `sha2` | 0.11.0 | MIT OR Apache-2.0 | 1.85 | identity fingerprint; the hash of HKDF for the vault payload key | none; no default features |
| `subtle` | 2.6.1 | BSD-3-Clause | not declared | constant-time comparison of invitation capabilities | no default features |
| `snow` | 0.10.0, pinned exactly | Apache-2.0 OR MIT | 1.85 | the Noise state machine of the session handshake and transport | none; no default features |
| `x25519-dalek` | 3.0.0, pinned exactly | BSD-3-Clause | 1.85 | X25519 for the session handshake | `static_secrets`, `zeroize`; no default features |
| `chacha20poly1305` | 0.11.0, pinned exactly | Apache-2.0 OR MIT | 1.85 | the session cipher; XChaCha20-Poly1305 of the vault | `zeroize`; no default features |
| `getrandom` | 0.3.4 | MIT OR Apache-2.0 | 1.63 | the random source of the operating system | none; no default features |
| `zeroize` | 1.9.0 | Apache-2.0 OR MIT | 1.85 | erasing key bytes on their way into a key type, and invitation capabilities | none; no default features. The desktop crates add `alloc`. |
| `sha3` | 0.11.0 | MIT OR Apache-2.0 | 1.85 | the checksum of an onion address | none; no default features |
| `tokio` | 1.53.1 | MIT | 1.71 | the runtime of the Tor adapter: sockets, timeouts, channels | `io-util`, `net`, `sync`, `time`; no default features. Tests add `rt` and `test-util`; the desktop adds `macros`. |
| `hmac` | 0.13.0 | MIT OR Apache-2.0 | 1.85 | HMAC-SHA256 of SAFECOOKIE control authentication | none; no default features |
| `argon2` | 0.6.0, pinned exactly | MIT OR Apache-2.0 | 1.85 | Argon2id, the key-encryption key of the vault from the passphrase | `alloc`, `zeroize`; no default features |
| `hkdf` | 0.13.0, pinned exactly | MIT OR Apache-2.0 | 1.85 | HKDF-SHA256, the payload key of the vault from the vault key | none; no default features |
| `rustix` | 1.1.5 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | 1.65 | the vault directory: `openat` without following links, `fstat`, the user id, `flock`, `fsync` of the directory | `std`, `fs`, `process`; no default features |
| `unicode-normalization` | 0.1.25, pinned exactly | MIT OR Apache-2.0 | 1.36 | NFC of the vault passphrase | none; no default features |

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
- Enabled by `monolith-session`: `zeroize`, so that a hash state is
  cleared when dropped. The session handshake derives its keys through
  HMAC-SHA256, and such a state holds key-dependent data. Not enabled:
  `alloc`, `oid`.
- Advisories: RUSTSEC-2021-0100, fixed long before 0.10.
- Unsafe: 50 lines, in the CPU-specific compression back ends.
- Also the hash of the session handshake: `Noise_XK_25519_ChaChaPoly_SHA256`.

`subtle`

- Maintained by the dalek-cryptography organization. Already in the tree
  through `ed25519-dalek` and `curve25519-dalek`; adding it as a direct
  dependency adds no code to the build.
- Monolith enables none of its features. `curve25519-dalek` enables
  `const-generics`, so that one is on in the build.
- Review: in the main scope of the 2019 Quarkslab audit of the dalek
  libraries.
- Advisories: none.
- Unsafe: 2 uses. One is the volatile read that keeps the optimizer from
  turning the comparison back into an early exit; the other converts a
  constant-time ordering result.
- Used for one thing: equality of `InvitationCapability`, a fixed-size
  16-byte value. It is a direct dependency of `monolith-protocol`, declared
  as such and not relied on through `ed25519-dalek`, because a hand-written
  comparison loop has no such barrier against the optimizer.
- What it gives is a best-effort software property: no early exit and a
  barrier against the optimizer restoring one. It is not a guarantee that
  timing side channels are impossible on every compiler and processor.

`snow`

- The Noise Protocol Framework in Rust: handshake state, symmetric state
  and cipher states. Monolith uses one pattern, `XK`, with one suite.
- Maintained by one person, as a "reasonable-effort" project. 0.10.0 was
  released on 2025-07-19 and is the newest release.
- Features: none. Every default feature is off. In particular the stock
  crypto resolver (`default-resolver-crypto`) is not compiled, so none of
  the cryptographic crates `snow` would bring is in the tree, and neither
  is `ring`. The primitives come from the resolver in `monolith-session`.
- Review: Trail of Bits, January 2024, for AgileBits, on an earlier
  commit. Ten findings; eight were fixed. Two remain open. TOB-SNOW-8:
  key material is not cleared. TOB-SNOW-2: an ephemeral key in a
  pre-message is not mixed into the key; the XK pattern has no ephemeral
  pre-message and Monolith uses no pre-shared key, so it does not apply.
  Monolith's own composition, which is the resolver and the binding to
  identities, was not part of that review and has had no external review
  at all.
- Advisories: RUSTSEC-2024-0011. A message that failed authentication
  still advanced the receiving nonce of the transport state, so that
  someone who could inject bytes could stop delivery. Fixed in 0.9.5 and
  not applicable to 0.10.0. In Monolith a frame that fails authentication
  ends the session in any case.
- Unsafe: none; the crate forbids it.
- What it does not do, and Monolith does around it: it checks nothing
  about a public key and accepts an all-zero Diffie-Hellman result; it
  clears no memory. The first is enforced in the resolver
  (`PROTOCOL.md` section 10.2). The second is reduced by the resolver's
  key types and otherwise stated as a limit (`CRYPTOGRAPHY.md` section 8).
- The test hook `fixed_ephemeral_key_for_testing_only` of its builder is
  not called anywhere in Monolith. Fixed ephemeral keys for tests come
  from the resolver's random source in test and fuzz builds.

`x25519-dalek`

- Maintained by the dalek-cryptography organization. 3.0.0 was released on
  2026-07-06, together with `curve25519-dalek` 5.0.0, which it is a thin
  layer over.
- Enabled: `static_secrets` (a private key type that can be used for more
  than one exchange, which the transport key needs) and `zeroize` (private
  keys and shared secrets are wiped when dropped). Not enabled:
  `precomputed-tables` (already on in `curve25519-dalek` through
  `ed25519-dalek`), `getrandom`, `reusable_secrets`, `serde`.
- Review: the 2019 Quarkslab audit of the dalek libraries took what it
  calls a marginal look at `x25519-dalek`. That predates the 2.x and 3.x
  lines. No later public audit is known.
- Advisories: none.
- Unsafe: none.
- It depends on `rand_core` 0.10 for the signatures of its key generation
  functions. Monolith does not call them: keys are built from bytes that
  come from `getrandom`.

`chacha20poly1305`

- Maintained by RustCrypto. 0.11.0 was released on 2026-06-28.
- Enabled: `zeroize` (the cipher key is wiped when the cipher is dropped).
  Not enabled: `alloc`, `getrandom`, `rand_core`, `arrayvec`, `bytes`,
  `reduced-round`.
- Review: NCC Group audited the 0.3 release in 2020 and found no
  significant issue. That is several major versions ago. No later public
  audit is known.
- Advisories: none for this crate. RUSTSEC-2019-0029 concerned a counter
  overflow in `chacha20` 0.2 and was fixed in 0.2.3.
- Unsafe: none in this crate. `chacha20` and `poly1305` contain it in
  their vectorized back ends; see the table below.
- Used with the 96-bit nonce that Noise defines: 32 zero bits and a 64-bit
  counter. The nonce is formed in one place, in the resolver.

`getrandom`

- Maintained by the rust-random organization. 0.3.4 was released on
  2025-10-14. 0.4 exists; 0.3 is used because the test tooling already has
  it in the tree and nothing in 0.4 is needed.
- Features: none. On Linux it calls `getrandom(2)` and falls back to
  `/dev/urandom` on kernels that lack it; on Linux it depends on `libc`.
- Advisories: none.
- Unsafe: 91 lines, in the per-platform back ends that call the operating
  system.
- The only source of randomness in Monolith (S17): ephemeral handshake
  keys and generated transport keys. An error from it fails the operation.

`zeroize`

- Maintained by RustCrypto. Already in the tree through `ed25519-dalek`.
- Used directly for two things. In `monolith-session`, the buffer that
  carries 32 random bytes into a key type is wiped when it goes out of
  scope. In `monolith-protocol`, an `InvitationCapability` overwrites its
  bytes when it is dropped, through the `Zeroize` trait and a `Drop` impl
  written by hand; the derive macros are not used, so the crate brings in
  nothing else.
- The desktop crates enable `alloc` for `Zeroizing<String>`: the
  passphrase fields and the card texts the client holds are wiped when
  they are dropped (S62).
- What it gives is best effort. It clears the memory a value occupies when
  the value is dropped. It does not reach copies the compiler made, and it
  is not a guarantee against every way a secret can stay in memory.

`tokio`

- Maintained by the Tokio project; the most used async runtime for Rust.
  Added in Phase 3 for `monolith-tor`, the one crate that opens sockets.
- Features: `net` (TCP and Unix sockets), `io-util`, `time` (every
  timeout of `RESOURCE_LIMITS.md` section 4), `sync` (bounded channels of
  the mock backend). Not enabled: `fs`, `process` (nothing may start a
  process), `signal`, `rt-multi-thread`. The runtime itself is built by
  the binary.
- The desktop enables `macros`, for `select!` in the bridge between the
  window and the runtime. It brings `tokio-macros`, which is built with
  `syn` 3 while the core is built with `syn` 2. Both are build-time only,
  and the window stack brings `syn` 3 in any case (section 4). The core
  crates and the `monolith` binary do not enable `macros`.
- Advisories: none for 1.53.1. Unsafe: much, in the I/O driver and the
  scheduler; it is the runtime every networked Rust program of this kind
  relies on, and the alternative is writing one.
- Pulls in `mio`, `socket2` and `libc` for the system calls, and `bytes`
  and `pin-project-lite`.

`hmac`

- Maintained by RustCrypto, the same generation as `sha2` 0.11, so no
  second `digest`. HMAC is not written by hand.
- Used for the two HMAC-SHA256 values of SAFECOOKIE. No unsafe code.

`sha3`

- Maintained by RustCrypto, in the same generation as `sha2` 0.11 (`digest`
  0.11), so it adds no second `digest`. Added in Phase 3.
- Used for one thing: the two checksum bytes of an Onion Service v3
  address, SHA3-256 over `.onion checksum`, the key and the version, as
  the onion service specification defines it. `monolith-identity` builds
  and parses ServiceIDs with it; every ServiceID Tor returns is checked.
- Advisories: none. Unsafe: one line, in the reader of the hash state.
  The Keccak permutation is in `keccak`, whose unsafe code is the ARMv8
  SHA3 instruction back end only; the portable permutation has none.

`argon2`

- Maintained by RustCrypto, the implementation of RFC 9106 in the same
  family as the AEAD and hash crates already in use. Taken in the current
  generation (section 2).
- Enabled: `alloc` (the memory of the derivation is allocated on the
  heap) and `zeroize` (that memory is cleared when the derivation ends).
  Not enabled: the defaults `password-hash` (the PHC string format) and
  `getrandom`, and `kdf`, `parallel`, `rand_core`. `password-hash` and
  `phc` are listed in `Cargo.lock` as optional dependencies of `argon2`
  and are not compiled.
- Used only through `Argon2::new(Argon2id, V0x13, params)` and
  `hash_password_into`, with parameters bounded before the call
  (`STORAGE.md` section 3.3).
- Advisories: none. Unsafe: 13 lines, in the AVX2 block function, which
  is selected at run time through `cpufeatures`, and in the allocation of
  the zeroed block memory.

`hkdf`

- Maintained by RustCrypto. Used once: extract with an empty salt and
  expand with the label of `STORAGE.md` section 3.2, over `sha2`.
- Advisories: none. Unsafe: none.

`rustix`

- Maintained by the Bytecode Alliance. A safe interface to the system
  calls of the vault directory that the standard library does not offer
  at the minimum Rust version without `unsafe` in Monolith: opening a file
  relative to a directory handle with `O_NOFOLLOW`, `fstat` of a handle,
  the user id for the owner check, `flock` for the single-instance lock,
  and `fsync` of a directory. Monolith keeps `unsafe_code = "forbid"`.
- Enabled: `std`, `fs`, `process` (for `getuid`). On Linux it uses its
  raw system call back end (`linux-raw-sys`), not `libc`.
- Advisories: none in the RustSec database. Unsafe: about 1650 lines,
  as the crate exists to wrap system calls; the raw back end is its
  largest part.

`unicode-normalization`

- Maintained by the unicode-rs project. Used for one thing: the passphrase
  of a vault is normalized to NFC before Argon2id, so that one passphrase
  typed on two systems opens the same vault. The protocol never
  normalizes text (`PROTOCOL.md` section 9).
- Advisories: none. Unsafe: 5 lines, in lookup tables.

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
| `keccak` | 0.2.2 | Apache-2.0 OR MIT | `sha3` | 11 |
| `mio` | 1.2.3 | MIT | `tokio` | 176 |
| `socket2` | 0.6.5 | MIT OR Apache-2.0 | `tokio` | 247 |
| `bytes` | 1.12.1 | MIT | `tokio` | 151 |
| `pin-project-lite` | 0.2.17 | Apache-2.0 OR MIT | `tokio` | 20 |
| `cpufeatures` | 0.3.1 | MIT OR Apache-2.0 | `sha2`, `curve25519-dalek` | 11 |
| `cfg-if` | 1.0.5 | MIT OR Apache-2.0 | several | 0 |
| `chacha20` | 0.10.2 | MIT OR Apache-2.0 | `chacha20poly1305` | 66 |
| `poly1305` | 0.9.1 | Apache-2.0 OR MIT | `chacha20poly1305` | 50 |
| `aead` | 0.6.1 | MIT OR Apache-2.0 | `chacha20poly1305` | 0 |
| `cipher` | 0.5.2 | MIT OR Apache-2.0 | `chacha20` | 0 |
| `inout` | 0.2.2 | MIT OR Apache-2.0 | `aead`, `cipher` | 29 |
| `universal-hash` | 0.6.1 | MIT OR Apache-2.0 | `poly1305` | 0 |
| `ctutils` | 0.4.2 | Apache-2.0 OR MIT | `universal-hash` | 0 |
| `cmov` | 0.5.4 | Apache-2.0 OR MIT | `ctutils` | 25 |
| `rand_core` | 0.10.1 | MIT OR Apache-2.0 | `x25519-dalek` | 0 |
| `libc` | 0.2.189 | MIT OR Apache-2.0 | `getrandom`, `mio`, `socket2`, on Linux | bindings |
| `blake2` | 0.11.0 | MIT OR Apache-2.0 | `argon2` | 1 |
| `base64ct` | 1.8.3 | Apache-2.0 OR MIT | `argon2` | 5 |
| `linux-raw-sys` | 0.12.1 | Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT | `rustix`, on Linux | bindings |
| `bitflags` | 2.13.2 | MIT OR Apache-2.0 | `rustix` | 3 |
| `errno` | 0.3.14 | MIT OR Apache-2.0 | `rustix` | 12 |
| `tinyvec` | 1.13.3 | Zlib OR Apache-2.0 OR MIT | `unicode-normalization` | 5 |

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

The unsafe code of `chacha20` and `poly1305` is in the back ends for
particular processors, which are selected at run time through
`cpufeatures`. `inout` holds the pointer pairs that let a cipher work in
place. `cmov` is the conditional move behind the constant-time comparison
of authentication tags.

`rand_core` is a crate of traits. It contains no generator, and Monolith
uses none of it.

`blake2` is the hash inside Argon2; RUSTSEC-2019-0019 (HMAC-BLAKE2 with
long keys) was fixed in 0.8.1 and does not apply to 0.11.0. `base64ct` is the constant-time Base64
of the password-hash string format, which Monolith does not use; `argon2`
depends on it unconditionally. `linux-raw-sys` holds the generated
constants and structures of the Linux system call interface for `rustix`.

`snow` itself brings `subtle`, which was in the tree already, and
`rustc_version` for its build script.

Build-time only: `proc-macro2`, `quote`, `syn`, `unicode-ident` (for
`curve25519-dalek-derive`), `rustc_version`, `semver` (build script of
`curve25519-dalek`).

### Development only

| Crate | Version | Licence | MSRV | Used for |
| --- | --- | --- | --- | --- |
| `proptest` | 1.11.0 | MIT OR Apache-2.0 | 1.85 | property tests |

`monolith-protocol` also lists `ed25519-dalek` as a development
dependency. One test needs point arithmetic to build a key with a torsion
component. Since Phase 5 `monolith-core` uses `proptest` too, for the
models of the message state and of the runtime; no crate was added to the
lock file for it. The crate `monolith-runtime` of Phase 5 depends only on
crates of the workspace and on `tokio`, which was there.

`proptest` brings a random number stack (`rand` 0.9, `rand_core` 0.9 and
others) into test builds. None of it is linked into a release binary. The
fuzz targets under `fuzz/` are a separate cargo project with their own
lock file and use `libfuzzer-sys`.

### Desktop client: direct

Dependencies of `monolith-desktop`, `monolith-ui`, `monolith-ui-kit`,
`monolith-ui-model` and `monolith-ui-snapshot`, added in Phase 6
(ADR 0010, ADR 0012).

| Crate | Version | Licence | MSRV | Used for | Features |
| --- | --- | --- | --- | --- | --- |
| `egui` | 0.32.3, pinned exactly | MIT OR Apache-2.0 | 1.85 | the user interface: layout, input, text, painting | no default features; `eframe` adds `accesskit`, `bytemuck`, `log` |
| `eframe` | 0.32.3, pinned exactly | MIT OR Apache-2.0 | 1.85 | the window, its OpenGL context and the event loop | `glow`, `x11`, `wayland`, `accesskit`; no default features |
| `qrcodegen` | 1.8.0, pinned exactly | MIT | not declared | QR codes of cards, made locally (ADR 0011) | none; no default features |
| `jiff` | 0.2.38, pinned exactly | Unlicense OR MIT | 1.70 | dates and times in the system's time zone | `std`, `tz-system`, `tzdb-zoneinfo`, `tzdb-bundle-platform`; no default features |
| `unicode-segmentation` | 1.13.3, pinned exactly | MIT OR Apache-2.0 | 1.85 | grapheme clusters for the rendering limits (`RESOURCE_LIMITS.md` section 10) | no default features |
| `png` | 0.18.1 | MIT OR Apache-2.0 | 1.73 | PNG files of the software renderer, for screenshots and visual tests | no default features |

`egui` and `eframe`

- Maintained by the egui project. 0.32.3 was released on 2025-09-12. It
  is the newest release that builds with Rust 1.85; later releases need a
  newer compiler (`epaint` 0.36.2 declares 1.95). ADR 0010 records the
  choice.
- Not enabled: `default_fonts` (the fonts are Monolith's, ADR 0012),
  `persistence` (Monolith keeps the window size and its preferences
  itself, ADR 0011), `wgpu`, `web_screen_reader`. `wgpu`, `naga` and the
  rest of the `wgpu` back end are listed in `Cargo.lock` as optional
  dependencies of `eframe` and are not compiled.
- Whatever its features, `eframe` enables `egui-winit` with `clipboard`
  (`arboard`, and `smithay-clipboard` on Linux) and `links`
  (`webbrowser`), and links `image` for window icons. `webbrowser` and
  `image` are banned crates; see "Exceptions in deny.toml" below.
- Review: no public audit is known.
- Advisories: none for these crates. Their text goes through
  `ttf-parser`, which is unmaintained (RUSTSEC-2026-0192, below).
- Unsafe: none in `egui`, `epaint`, `emath` and `ecolor`; 24 lines in
  `eframe`, 35 in `egui_glow`, 1 in `egui-winit`. Most of the unsafe code
  of the client is further down, in the bindings to the system (table
  below).
- Size: counted with `cargo tree -e normal,build`, a Linux build of the
  desktop has 259 third-party crates and a Windows build 176, against 52
  for the `monolith` binary on Linux. All of it is statically linked Rust
  that Monolith ships and has to keep up to date (ADR 0010).

`qrcodegen`

- Project Nayuki's QR Code generator, in its Rust version. 1.8.0 was
  released on 2022-04-17 and is the newest release. It has no
  dependencies, no features and no unsafe code.
- Used in `monolith-ui-kit` to turn the text of a card into a matrix of
  modules, which the kit draws. It only encodes; the client reads no QR
  code.
- Advisories: none.

`jiff`

- Maintained by Andrew Gallant. 0.2.38 was released on 2026-10-06.
- Used in one place: a timestamp is shown as a date and time in the
  system's time zone. If the zone cannot be read, UTC is used.
- Enabled: `std`; `tz-system` (which zone the system uses);
  `tzdb-zoneinfo` (the zone database under `/usr/share/zoneinfo` on
  Linux); `tzdb-bundle-platform` (a copy of the database compiled in,
  through `jiff-tzdb-platform` and `jiff-tzdb`, on Windows, which has none
  in that form). Not enabled: the defaults `tz-fat`, `tzdb-concatenated`
  and `perf-inline`, and `serde`, `logging`, `static`. `jiff-static`, a
  procedural macro of the crate, is compiled at build time.
- Advisories: none. Unsafe: 79 lines.

`unicode-segmentation`

- Maintained by the unicode-rs project. Already in the tree through
  `egui` and `winit`. `monolith-ui-model` depends on it directly to count
  grapheme clusters for the rendering limits.
- Advisories: none. Unsafe: none.

`png`

- Maintained by the image-rs project. `monolith-ui-snapshot`, the software
  renderer of egui frames, uses it to write PNG files and to read back the
  reference images it wrote, for the visual tests.
- `monolith-ui-snapshot` is a development dependency, and the optional
  `screenshot` feature of the desktop, which no release enables. A release
  build links `png` all the same, through `image` under `eframe` (below).
- Advisories: none. Unsafe: none. It brings `fdeflate`, `flate2`,
  `miniz_oxide` and `crc32fast`.

### Desktop client: transitive

The notable crates under the ones above. `cargo tree -p monolith-desktop`
gives the full list.

| Crate | Version | Licence | Through | Unsafe lines |
| --- | --- | --- | --- | --- |
| `epaint`, `emath`, `ecolor` | 0.32.3 | MIT OR Apache-2.0 | `egui` | 0 |
| `egui-winit` | 0.32.3 | MIT OR Apache-2.0 | `eframe` | 1 |
| `egui_glow` | 0.32.3 | MIT OR Apache-2.0 | `eframe` | 35 |
| `winit` | 0.30.13 | Apache-2.0 | `eframe` | 918 |
| `glutin` | 0.32.3 | Apache-2.0 | `eframe` | 342 |
| `glutin-winit` | 0.5.0 | MIT | `eframe` | 2 |
| `glow` | 0.16.0 | MIT OR Apache-2.0 OR Zlib | `eframe`, `egui_glow` | 2635, bindings |
| `libloading` | 0.8.9 | ISC | `glutin`, `x11rb` | 49 |
| `x11rb` | 0.13.2 | MIT OR Apache-2.0 | `winit`, `arboard`, on Linux | 66 |
| `x11-dl` | 2.21.0 | MIT | `winit`, `glutin`, on Linux | bindings |
| `wayland-client` | 0.31.15 | MIT | `winit`, on Linux | 1 |
| `wayland-backend` | 0.3.17 | MIT | `wayland-client` | 212 |
| `smithay-client-toolkit` | 0.19.2 and 0.20.0 | MIT | `winit`; `smithay-clipboard` | 18 |
| `ab_glyph` | 0.2.32 | Apache-2.0 | `epaint` | 0 |
| `ab_glyph_rasterizer` | 0.1.10 | Apache-2.0 | `ab_glyph` | 5 |
| `owned_ttf_parser` | 0.25.1 | Apache-2.0 | `ab_glyph` | 5 |
| `ttf-parser` | 0.25.1 | MIT OR Apache-2.0 | `owned_ttf_parser` | 0 |
| `ahash` | 0.8.12 | MIT OR Apache-2.0 | `egui`, `epaint`, `winit` | 7 |
| `arboard` | 3.6.1 | MIT OR Apache-2.0 | `egui-winit` | 53 |
| `smithay-clipboard` | 0.7.3 | MIT | `egui-winit`, on Linux | 6 |
| `clipboard-win` | 5.4.1 | BSL-1.0 | `arboard`, on Windows | 72 |
| `error-code` | 3.4.0 | BSL-1.0 | `clipboard-win` | 15 |
| `accesskit` | 0.19.0 | MIT OR Apache-2.0 | `egui` | 0 |
| `accesskit_winit` | 0.27.0 | Apache-2.0 | `egui-winit` | 3 |
| `accesskit_consumer` | 0.28.0 | MIT OR Apache-2.0 | the platform crates below | 0 |
| `accesskit_unix` | 0.15.0 | MIT OR Apache-2.0 | `accesskit_winit`, on Linux | 0 |
| `accesskit_atspi_common` | 0.12.0 | MIT OR Apache-2.0 | `accesskit_unix` | 0 |
| `atspi` | 0.25.0 | Apache-2.0 OR MIT | `accesskit_unix` | 0 |
| `zbus` | 5.13.2 | MIT | `accesskit_unix`, `atspi` | 54 |
| `async-executor` | 1.14.0 | Apache-2.0 OR MIT | `accesskit_unix`, `zbus` | 21 |
| `fastrand` | 2.5.0 | Apache-2.0 OR MIT | `async-executor`, `futures-lite`, `piper` | 0 |
| `zbus_xml` | 5.1.0 | MIT | `zbus-lockstep` under `atspi-common` | 0 |
| `quick-xml` | 0.38.4 | MIT | `zbus_xml` | 0 |
| `quick-xml` | 0.41.0 | MIT | `wayland-scanner`, build time | 0 |
| `accesskit_windows` | 0.27.0 | MIT OR Apache-2.0 | `accesskit_winit`, on Windows | 85 |
| `windows` | 0.61.3 | MIT OR Apache-2.0 | `accesskit_windows` | bindings |
| `webbrowser` | 1.2.4 | MIT OR Apache-2.0 | `egui-winit` | 8 |
| `url` | 2.5.8 | MIT OR Apache-2.0 | `webbrowser` | 1 |
| `image` | 0.25.9 | MIT OR Apache-2.0 | `eframe`, `arboard` | 6 |
| `jiff-tzdb-platform`, `jiff-tzdb` | 0.1.3, 0.1.9 | Unlicense OR MIT | `jiff`, on Windows | 0 |
| `fdeflate` | 0.3.7 | MIT OR Apache-2.0 | `png` | 0 |
| `flate2` | 1.1.10 | MIT OR Apache-2.0 | `png` | 36 |
| `miniz_oxide` | 0.8.9 and 0.9.1 | MIT OR Zlib OR Apache-2.0 | `png`; `flate2` | 0 |
| `crc32fast` | 1.5.2 | MIT OR Apache-2.0 | `png` | 15 |

- Window: `winit` opens the window and reads input. On Linux it talks to
  the X server through `x11rb` and `x11-dl`, or to the Wayland compositor
  through `wayland-client` and `smithay-client-toolkit`; on Windows it
  uses the Win32 API. `glutin` creates the OpenGL context (EGL or GLX on
  Linux, WGL on Windows) and loads the system's libraries at run time
  through `libloading`; `glow` holds the OpenGL bindings `egui_glow`
  paints with. These crates bind C interfaces of the system and hold most
  of the unsafe code of the client.
- Text: `epaint` lays out text with `ab_glyph`, which reads fonts through
  `owned_ttf_parser` and `ttf-parser`.
- Clipboard: `arboard`, with `smithay-clipboard` on Wayland and
  `clipboard-win` on Windows. `egui-winit` reads only text from it, and
  writes an image only when an egui command asks to copy one, which the
  client never issues.
- Accessibility: AccessKit publishes the widget tree. On Linux
  `accesskit_unix` speaks AT-SPI (`atspi`) on the session bus through
  `zbus`; on Windows `accesskit_windows` uses UI Automation through the
  `windows` crate. On Linux this is the largest part of the tree
  (ADR 0010). `zbus` runs on the `async-io` executor stack
  (`async-executor`, `async-io`, `futures-lite`, `piper`, `blocking`),
  which `accesskit_winit` selects by default; `egui-winit` does not let it
  be changed.
- `webbrowser` opens a URL in the system's browser; on Linux it starts a
  process such as `xdg-open`. It brings `url`, and with it `idna` and the
  ICU crates. The client never lets it act (below).
- `ahash` hashes the maps of egui and winit. On Linux it seeds its keys
  from `getrandom` 0.3.4, the one in the tree.
- `smithay-client-toolkit` 0.19 and 0.20, and `miniz_oxide` 0.8 and 0.9,
  are second versions; section 4 lists them all.

### Exceptions in deny.toml

Each is written in `deny.toml` with its reason. The ignored advisories are
also ignored for `cargo audit` in `.cargo/audit.toml`.

Advisories ignored, to be reviewed on 2027-01-15:

- RUSTSEC-2026-0194 and RUSTSEC-2026-0195, `quick-xml` 0.38.4. The first
  makes the duplicate-attribute check of a start tag quadratic in the
  number of attributes; the second lets the namespace declarations of a
  start tag allocate without bound in `NsReader`. Both are fixed in
  0.41.0. This `quick-xml` comes only through `zbus_xml` 5.1.0, which
  requires 0.38. `zbus_xml` 5.1.1 moved to 0.39 and 5.2 parses XML
  without `quick-xml`, but both need Rust 1.87. `zbus_xml` serves
  `zbus-lockstep` and its procedural macro, which `atspi-common` uses to
  check that its message types match the XML interface descriptions
  shipped inside `atspi-common`: the macro parses those files when
  `atspi-common` is compiled, and the functions of `zbus-lockstep` are
  called only from that crate's tests. `zbus` itself does not depend on
  `zbus_xml`. No XML is parsed while the client runs, and none comes from
  a peer or from the session bus. `wayland-scanner` uses `quick-xml`
  0.41.0, which is fixed.
- RUSTSEC-2026-0192, `ttf-parser` 0.25.1: unmaintained; its author has
  said it will get no further fixes. No vulnerability is known. It is the
  font parser of `ab_glyph`, which `epaint` 0.32.3 uses for all text.
  Monolith gives it the fonts compiled into the binary, which the tests
  render, and the system fonts of the fixed list of ADR 0012, which are
  read only if they are no larger than 32 MiB and are first tried in a
  separate egui context, so that a font egui cannot read is left out. No
  font comes from a peer or from a file the user names. `epaint` 0.36
  replaces it with `skrifa` and needs Rust 1.95; raising the minimum Rust
  version is the way to drop it (ADR 0010).
- RUSTSEC-2026-0370, `zvariant` 5.9.2, and RUSTSEC-2026-0371,
  `zbus_names` 4.3.1 (2026-10-19, found by the final verification of
  Phase 7): unmaintained as crates of their own, since `zbus` 6 took them
  in as its `wire` and `names` modules. No vulnerability is known. They
  are the D-Bus types of `zbus` 5.13.2, which `accesskit_unix` 0.15 under
  `eframe` 0.32.3 uses on Linux to give the screens to assistive
  technology over the session bus; no peer reaches D-Bus, and nothing of
  them is used on Windows. `zbus` 6 needs Rust 1.89, above the minimum
  Rust version, and an `accesskit_unix` that uses it; neither `Cargo.lock`
  nor any code changed for these exceptions. `zvariant_derive` is not in
  the tree's findings and is not ignored.

Banned crates allowed only as a direct dependency of named crates
(`wrappers`); any other crate that depends on them still fails the check:

- `webbrowser`, under `egui-winit` only. `eframe` 0.32.3 enables the
  `links` feature of `egui-winit` in every build. The window removes every
  `OpenUrl` command from egui's output at the end of each frame, before
  `eframe` hands the output to `egui-winit`, and the client draws no
  hyperlink widget (ADR 0010, S61). The test
  `window::tests::no_url_leaves_a_frame` checks the filter; mutation fault
  UI16 removes it. Nothing that `webbrowser` brings is banned.
- `image`, under `eframe` and `arboard` only. `eframe` links its PNG code
  for window icons: it decodes its own default icon when the application
  sets none, and on Windows it scales and encodes the icon it is given.
  `arboard` links it (PNG on Linux, PNG and BMP on Windows) for images on
  the clipboard. The client builds its icon from pixels it computes
  (`window::icon`), never asks to copy an image, and `egui-winit` reads
  only text from the clipboard. No peer data and no file reaches the
  decoder (S16).
- `fastrand`, under `async-executor`, `futures-lite`, `piper` and
  `tempfile` only. The first three are the executor stack under `zbus` on
  Linux, which uses it to vary the order in which tasks are polled.
  `tempfile` is under `zbus` on Windows, where `zbus` is not built. No
  value of Monolith comes from it: its randomness comes from `getrandom`
  alone (S17). The executor cannot be swapped for `tokio`, because
  `egui-winit` enables `accesskit_winit` with its default `async-io`
  feature.

Licence added: BSL-1.0, the Boost Software License 1.0, for
`clipboard-win` and `error-code`, the Windows clipboard under `arboard`.
It is OSI approved and permissive: it asks for its notice in copies of
the source, not in binaries. Every other licence in the tree is either
allowed already or offered next to an allowed one (`Unlicense OR MIT` of
`jiff` and `memchr`, for example). The fonts under
`crates/monolith-ui-kit/assets` are not crates and `cargo deny` does not
see them; their licences (SIL Open Font License 1.1, ISC) are next to
them (ADR 0012).

Notices in the packages: most of these licences (MIT, Apache-2.0, the BSD
licences, ISC, Unicode-3.0) ask for the copyright notice and the licence
text in every copy, binaries included. Each archive of the desktop client
carries `THIRD-PARTY-NOTICES.txt`, written when it is built by
`packaging/notices.py` for the target built: the crates `cargo tree -p
monolith-desktop --edges normal` names for it, with their versions and
licences, every licence and notice file their packages carry, the
standard texts of SPDX in `packaging/licenses/` for the crates whose
packages carry none (the egui and AccessKit crates, `atspi-proxies`,
`clipboard-win`, `profiling`, `qrcodegen`) and for the Rust standard
library, and the licences of the fonts and icons (`packaging/README.md`).

## 4. Checks

Run on the dependency set above; the date is in each row.

| Check | Result |
| --- | --- |
| `cargo audit` | no vulnerability applies; RUSTSEC-2026-0194, RUSTSEC-2026-0195, RUSTSEC-2026-0192, RUSTSEC-2026-0370 and RUSTSEC-2026-0371 are ignored with their reasons (section 3, "Exceptions in deny.toml"); 445 crates in `Cargo.lock` (2026-10-14, Phase 6; the last two since 2026-10-19) |
| `cargo deny check` (advisories, bans, licences, sources) | passes with the repository policy, no warning (2026-10-14; again on 2026-10-19 with the two exceptions of that day) |
| `cargo tree -d` | in the core, one pair: `rand_core` 0.9.5 and 0.10.1; in the desktop, the second versions of its window stack; see below (2026-10-14) |
| Build and tests with Rust 1.85.1 | pass; after the Phase 6 changes to `Cargo.lock` the workspace was built with 1.85.1 on Windows (2026-10-14) |
| Highest declared MSRV in the tree | 1.85 (the dalek and RustCrypto crates, `snow`, `zeroize`, `proptest`; for the desktop `egui`, `eframe`, `zbus`, `zbus_xml`, `webbrowser`, `image`, `unicode-segmentation` and others) |

The highest declared MSRV equals the project's. A later release of any of
these crates may need a newer compiler; the resolver will then keep the
older version, and the MSRV job in CI fails if it cannot. This already
holds the egui crates at 0.32.3 (ADR 0010) and `zbus_xml` at 5.1.0, whose
successors need Rust 1.87 or later.

`Cargo.lock` lists every optional dependency that some feature could
enable, also those no build of Monolith compiles, such as the `wgpu` back
end of `eframe`. `cargo audit` reads the lock file and so checks them
too; `cargo deny` checks the graph that the features select, for every
target.

Duplicate versions. In the core, the library crates and the `monolith`
binary, `cargo tree -d` reports

    rand_core v0.9.5    <- rand, rand_chacha, rand_xorshift <- proptest (development only)
    rand_core v0.10.1   <- x25519-dalek <- monolith-session

and nothing else. No cryptographic implementation is in the tree twice:
there is one `curve25519-dalek`, one `sha2`, one `chacha20poly1305`, one
`digest`, one `getrandom`, one `zeroize`, one `subtle`.

The pair is accepted. `rand_core` is a crate of traits, 0.9.5 is only in
test builds, and the alternative would be to give up either property
tests or the maintained X25519 crate. `deny.toml` states the setting that
makes this pass, `multiple-versions-include-dev = false`: a crate that is
in the tree only through development dependencies is not counted as a
duplicate. A second version of any crate in a product build still fails
the check.

The desktop client adds second versions, all from its window stack.
Crates of that stack depend on different major versions of the same
crate, and Monolith cannot unify them without patching those crates.
`deny.toml` accepts each extra version by its exact version number in
`skip`, with a reason; a new version of any of these crates, or a second
version of any other crate, fails the check until it is reviewed. None of
them is cryptographic. Six of these crates are also in the core
(`rustix`, `linux-raw-sys`, `bitflags`, `syn`, `windows-sys`,
`windows-link`); for each, the version accepted is the window stack's and
never the one the core uses.

| Crate | Accepted in `skip` | Other version | Where the accepted one comes from |
| --- | --- | --- | --- |
| `rustix` | 0.38.44 | 1.1.5 | `winit` 0.30, `calloop` 0.13 and `smithay-client-toolkit` 0.19 on Linux |
| `linux-raw-sys` | 0.4.15 | 0.12.1 | `rustix` 0.38 |
| `smithay-client-toolkit` | 0.19.2 | 0.20.0 | `winit` 0.30; 0.20 is under `smithay-clipboard` |
| `calloop` | 0.13.0 | 0.14.4 | `winit` and `smithay-client-toolkit` 0.19 |
| `calloop-wayland-source` | 0.3.0 | 0.4.1 | `smithay-client-toolkit` 0.19 |
| `thiserror`, `thiserror-impl` | 1.0.69 | 2.0.21 | `calloop` 0.13, `smithay-client-toolkit` 0.19, `accesskit_atspi_common`, `ndk` |
| `hashbrown` | 0.15.5 | 0.17.1 | AccessKit; 0.17 is under `toml_edit`, at build time |
| `winnow` | 0.7.15 | 1.0.4 | `zbus` and `zvariant`; 1.0 is under `toml_edit`, at build time |
| `quick-xml` | 0.38.4 | 0.41.0 | `zbus_xml` (section 3); 0.41 is under `wayland-scanner`, at build time |
| `syn` | 3.0.9 | 2.0.119 | procedural macros of the window stack and `tokio-macros`, at build time |
| `miniz_oxide` | 0.9.1 | 0.8.9 | `flate2` 1.1.10 under `png`, which itself uses 0.8 |
| `windows-sys` | 0.52.0, 0.59.0, 0.60.2 | 0.61.2 | `glutin` and `winit`; `eframe` and `rustix` 0.38; `arboard` |
| `windows-targets` and the eight `windows_*` crates | 0.52.6 | 0.53.x | `windows-sys` 0.52 and 0.59; 0.53 is under `arboard`'s `windows-sys` 0.60 |
| `windows-link` | 0.1.3 | 0.2.1 | the `windows` 0.61 crates of `accesskit_windows` |

The check covers every target, so second versions that exist only on
platforms Monolith does not build for are accepted the same way:
`bitflags` 1.3.2 (`core-graphics` on macOS, `redox_syscall` 0.4 on
Redox), `objc2` 0.5.3, `objc2-app-kit` 0.2.2 and `objc2-foundation` 0.2.2
(macOS), `jni-sys` 0.3.1 (Android), `redox_syscall` 0.4.1 and 0.5.18
(Redox).

`getrandom` is held to one version through the lock file. `jobserver`
0.1.35 depends on `getrandom` 0.4 on Windows hosts. It is in the graph
only through the `parallel` feature of `cc`, which `android-activity`
enables for its build script, so only in Android builds. `Cargo.lock`
keeps `jobserver` at 0.1.34, which uses `getrandom` 0.3 like the rest of
the tree and which `cc` 1.8.0 accepts. A `cargo update` that raises it
makes `cargo deny` fail on `getrandom` until it is reviewed again.

## 5. Not yet chosen

- Storage: `argon2`, `hkdf`, and the message store: ADR 0005. The vault
  cipher is XChaCha20-Poly1305 from `chacha20poly1305`, which is in the
  tree for the session layer.
- Passphrase normalization for the vault (`STORAGE.md` section 3):
  `unicode-normalization` was used by the protocol for display names until
  normalization was removed from protocol validity, and was dropped with
  it. Whether the vault normalizes passphrases, and with what, is decided
  with storage.
- Onion address checksum: `sha3`, with the Tor backend.
- Runtime, logging: with the phases that need them. The GUI toolkit is
  egui with eframe (ADR 0010; section 3).

## 6. Sources

- https://crates.io/crates/ed25519-dalek/versions
- https://crates.io/crates/curve25519-dalek/versions
- https://rustsec.org/advisories/RUSTSEC-2022-0093.html
- https://rustsec.org/advisories/RUSTSEC-2024-0344.html
- https://blog.quarkslab.com/security-audit-of-dalek-libraries.html
- https://docs.rs/ed25519-dalek/3.0.0/ed25519_dalek/struct.VerifyingKey.html
- https://crates.io/crates/sha2/versions
- https://crates.io/crates/snow/0.10.0
- https://github.com/trailofbits/publications/blob/master/reviews/2024-03-agilebits-snow-securityreview.pdf
- https://rustsec.org/advisories/RUSTSEC-2024-0011.html
- https://crates.io/crates/x25519-dalek/3.0.0
- https://crates.io/crates/chacha20poly1305/0.11.0
- https://research.nccgroup.com/2020/02/26/public-report-rustcrypto-aes-gcm-and-chacha20poly1305-implementation-review/
- https://rustsec.org/advisories/RUSTSEC-2019-0029.html
- https://crates.io/crates/getrandom/0.3.4
- https://crates.io/crates/egui/0.32.3
- https://crates.io/crates/eframe/0.32.3
- https://crates.io/crates/qrcodegen/1.8.0
- https://crates.io/crates/jiff/0.2.38
- https://crates.io/crates/unicode-segmentation/1.13.3
- https://crates.io/crates/png/0.18.1
- https://crates.io/crates/zbus_xml/versions
- https://rustsec.org/advisories/RUSTSEC-2026-0194.html
- https://rustsec.org/advisories/RUSTSEC-2026-0195.html
- https://rustsec.org/advisories/RUSTSEC-2026-0192.html
- https://rustsec.org/advisories/RUSTSEC-2026-0370.html
- https://rustsec.org/advisories/RUSTSEC-2026-0371.html
- https://github.com/harfbuzz/ttf-parser/issues/217
- https://www.boost.org/LICENSE_1_0.txt
- https://embarkstudios.github.io/cargo-deny/checks/bans/cfg.html
