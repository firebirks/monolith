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
  the same crate in the product. A crate that is in the tree only through
  a development dependency is not counted; section 4 lists the one case.
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

Production dependencies of `monolith-identity`, `monolith-protocol` and
`monolith-session`.

### Direct

| Crate | Version | Licence | MSRV | Used for | Features |
| --- | --- | --- | --- | --- | --- |
| `ed25519-dalek` | 3.0.0, pinned exactly | BSD-3-Clause | 1.85 | identity signatures, key validation | `fast`, `zeroize`; no default features |
| `sha2` | 0.11.0 | MIT OR Apache-2.0 | 1.85 | identity fingerprint | none; no default features |
| `subtle` | 2.6.1 | BSD-3-Clause | not declared | constant-time comparison of invitation capabilities | no default features |
| `snow` | 0.10.0, pinned exactly | Apache-2.0 OR MIT | 1.85 | the Noise state machine of the session handshake and transport | none; no default features |
| `x25519-dalek` | 3.0.0, pinned exactly | BSD-3-Clause | 1.85 | X25519 for the session handshake | `static_secrets`, `zeroize`; no default features |
| `chacha20poly1305` | 0.11.0, pinned exactly | Apache-2.0 OR MIT | 1.85 | the session cipher | `zeroize`; no default features |
| `getrandom` | 0.3.4 | MIT OR Apache-2.0 | 1.63 | the random source of the operating system | none; no default features |
| `zeroize` | 1.9.0 | Apache-2.0 OR MIT | 1.85 | erasing key bytes on their way into a key type | none; no default features |

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
- Used directly for one thing: the buffer that carries 32 random bytes
  into a key type is wiped when it goes out of scope.
- What it gives is best effort. It clears the memory a value occupies when
  the value is dropped. It does not reach copies the compiler made, and it
  is not a guarantee against every way a secret can stay in memory.

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
| `chacha20` | 0.10.2 | MIT OR Apache-2.0 | `chacha20poly1305` | 66 |
| `poly1305` | 0.9.1 | Apache-2.0 OR MIT | `chacha20poly1305` | 50 |
| `aead` | 0.6.1 | MIT OR Apache-2.0 | `chacha20poly1305` | 0 |
| `cipher` | 0.5.2 | MIT OR Apache-2.0 | `chacha20` | 0 |
| `inout` | 0.2.2 | MIT OR Apache-2.0 | `aead`, `cipher` | 29 |
| `universal-hash` | 0.6.1 | MIT OR Apache-2.0 | `poly1305` | 0 |
| `ctutils` | 0.4.2 | Apache-2.0 OR MIT | `universal-hash` | 0 |
| `cmov` | 0.5.4 | Apache-2.0 OR MIT | `ctutils` | 25 |
| `rand_core` | 0.10.1 | MIT OR Apache-2.0 | `x25519-dalek` | 0 |
| `libc` | 0.2.189 | MIT OR Apache-2.0 | `getrandom`, on Linux | bindings |

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
component.

`proptest` brings a random number stack (`rand` 0.9, `rand_core` 0.9 and
others) into test builds. None of it is linked into a release binary. The
fuzz targets under `fuzz/` are a separate cargo project with their own
lock file and use `libfuzzer-sys`.

## 4. Checks

Run on the dependency set above on 2026-10-02:

| Check | Result |
| --- | --- |
| `cargo audit` | no advisory applies; 60 crates in `Cargo.lock` |
| `cargo deny check` (advisories, bans, licences, sources) | passes with the repository policy |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 and 0.10.1, see below |
| Build and tests with Rust 1.85.1 | pass |
| Highest declared MSRV in the tree | 1.85 (the dalek and RustCrypto crates, `snow`, `zeroize`, `proptest`) |

The highest declared MSRV equals the project's. A later release of any of
these crates may need a newer compiler; the resolver will then keep the
older version, and the MSRV job in CI fails if it cannot.

Duplicate versions. `cargo tree -d` reports

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
- Runtime, logging, GUI: with the phases that need them.

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
