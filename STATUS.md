# Status of the Phase 2 work

Written on 2026-10-02 so that the work can be picked up on another machine.
This file describes work in progress on the branch `phase-2-session-crypto`.
It is removed when Phase 2 is closed; it does not belong on `main`.

Branch: `phase-2-session-crypto`, on top of `main` at `520e91e`.
Last commit of work before this file: `e194dca`.

## 1. What Phase 2 is

The authenticated encrypted session between two peers: the handshake, the
binding of its keys to Monolith identities, the encrypted frames, the
session limits, and the tests that go with them. No network I/O, no Tor,
no storage, no user interface. Phase 3 is not started.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not rewritten or moved.
- Nothing is pushed unless the owner asks for it.
- A wire format of Phase 1 changes only with a written reason, updated
  vectors and a statement of the compatibility impact.
- `native-tls` and `openssl-sys` stay banned in `deny.toml`.
- The specification comes first. If code and `docs/` disagree, the
  documents win.

## 2. Decisions taken

All of them are recorded in `docs/adr/0002-session-protocol.md`.

- Session layer: `Noise_XK_25519_ChaChaPoly_SHA256` (Noise revision 34),
  one suite, nothing negotiated. TLS 1.3 with raw public keys was the
  other finalist and was not chosen.
- Every identity has three keys: identity (Ed25519, signs contact cards
  only), transport (X25519, the Noise static key), onion (Ed25519, Tor).
- The contact card states the transport key and is the certificate for
  it. Card version 1 is 171 bytes, 187 with an invitation capability.
- Prologue: `MONOLITH-SESSION-V1` followed by the responder's identity
  key. No preamble, no version field, no feature bits.
- Handshake messages are 48, 48 and 235 bytes. The third carries the
  initiator's card.
- There is no identity proof message. Code 0x0001 is not assigned.
- Library: `snow` 0.10.0 without its default features, with a resolver of
  our own over `x25519-dalek` 3.0.0, `chacha20poly1305` 0.11.0, `sha2`
  0.11.0 and `getrandom` 0.3 (F-R1 in the ADR).
- The stale-card rule: a card older than the newest card held of an
  identity, or contradicting it, does not open a contact session.

## 3. What is done

Commits on the branch, oldest first:

| Commit | Content |
| --- | --- |
| `6061073`, `626daec` | First decision for TLS 1.3 and its profile. Superseded. |
| `475937a` | Decision reopened with Noise XK as second finalist. |
| `79255ca` | Noise XK selected; `PROTOCOL.md`, `CRYPTOGRAPHY.md` and the ADR rewritten for it, with independent test vectors. |
| `f677968` | Crypto provider chosen (F-R1). |
| `bb5e493` | `TransportPublicKey` in `monolith-identity`: canonical encoding, small-order points, Montgomery form check. |
| `81d32ff` | Contact card with the transport key; key separation between the three keys; seeds regenerated. |
| `ae8c865` | AuthProof and preamble removed; session logic takes authentication from the handshake; `Standing::StaleCard`, `PeerRecord`, `Admission`. |
| `159c6cf` | New crate `monolith-session`: resolver, typed handshake, `AuthenticatedSession`, tests, dependency records. |
| `e68c35d` | Fuzz targets `handshake_responder`, `handshake_initiator`, `session_frames`, with generated seeds. |
| `2eb09ad` | More handshake tests: another Noise pattern, a pinned card with an invitation, an older pinned card. |
| `6c67166` | Documents brought in line: threat model, invariants, resource limits, test plan, architecture and others. |
| `bf2f377` | Review fix: stale-card rule compares with the newest card held, pending or pinned, and is applied by the initiator as well. |
| `e194dca` | Review fixes: session limits, sending rules, key erasure at the end of a session, small items. |

Where things are:

- `crates/monolith-identity/src/transport.rs`: X25519 public key validity.
- `crates/monolith-protocol/src/card.rs`: card format and `evaluate_card`.
- `crates/monolith-protocol/src/session.rs`: session logic, standing table.
- `crates/monolith-session/src/resolver.rs`: the primitives handed to
  `snow`; the only place where rule F5 is applied.
- `crates/monolith-session/src/handshake.rs`: `HandshakeInitiator`,
  `HandshakeResponder`, `HandshakeResponderFinal`, `OutboundPeer`,
  `InboundPeer`, `MessageBuffer`.
- `crates/monolith-session/src/session.rs`: `AuthenticatedSession`.
- `crates/monolith-session/src/tests/`: `vectors` (known answers),
  `handshake` (hostile peer), `frames`, `contacts` (two real sessions and
  the bytes between them), `properties`, `seeds`.

## 4. What was verified, and on which commit

Be careful with this table: several checks were run before the review
fixes and have to be run again on the final commit.

| Check | Result | Run on |
| --- | --- | --- |
| `cargo test --workspace --locked`, Rust 1.95.0 | 371 tests pass | `e194dca` |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | clean | `e194dca` |
| `cargo fmt --all --check` | clean | `e194dca` |
| Fuzz targets compile with `--cfg fuzzing` | yes | `e194dca` |
| `cargo +1.85.1 test --workspace --locked` | pass | `159c6cf`, before the review fixes |
| `cargo deny check`, `cargo audit` | pass | `159c6cf` |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 (tests only) and 0.10.1 | `159c6cf` |
| Mutation list of Phase 2, 78 faults | all caught | `e68c35d` |
| Fuzz smoke run, 11 targets, 25 seconds each | no failure | `6c67166` |
| Known-answer vectors of `PROTOCOL.md` 16.1 | reproduced byte for byte | every commit since `159c6cf` |
| Handshake cost measurements in `RESOURCE_LIMITS.md` 11.1 | measured | `6c67166` |

## 5. The two review passes

Two independent reviews were made on `6c67166`: one of the cryptographic
protocol and authentication semantics, one of hostile implementation
behavior. Neither found a critical or high issue.

Fixed:

- Stale-card rule compared with the pinned card only, so a retired key
  kept working while a newer card waited for the user. Now the newest
  card held counts, and `OutboundPeer::admit` takes the record too.
- A Close sent at the age limit was a violation for the peer. Receiving
  now allows `SESSION_CLOSE_GRACE` (120 s) past the age limit.
- Locally lowered limits were enforced against the peer. The limits are
  now constants; the constructor for lower values is test-only.
- `set_transfer_active(true)` revived an expired session. It now takes
  the time and refuses once the ordinary lifetime is reached.
- Application messages could be sent before the local ContactAccept.
- More than one ContactRequest could be sent on a session.
- The invitation capability in an outgoing request was not bound to the
  peer that is asked.
- Cipher keys were erased when the session object was dropped, not when
  the session ended.
- `TransportSecretKey::from_bytes` accepted 32 zero bytes.
- Session limits were missing from `PROTOCOL.md`; they are section 7.1.
- Wording: budgets by standing, the claim about `Session`, the key
  lifetime table, the `Expired` error, a comment about signatures.
- Tests: exact `Debug` output of a session, a failing random source, the
  private transport keys in the "nothing leaks" check.
- `monolith-cli` refuses to compile with `--cfg fuzzing`.

Accepted as documented limits: `Instant` does not advance during suspend
on Linux; an identity can name an endpoint it does not own (threat model,
adversary B); the `Internal` path of `seal` is not reachable from a test.

Still open from the reviews: the fuzz target items in section 6.

## 6. What is left, in order

### 6.1 Fuzz targets

`fuzz/fuzz_targets/session_frames.rs`:

- Remove the final loop `delivered <= sent.len()`; it cannot fail.
- Let time pass. Keep a clock in `World` and pass it to every `send` and
  `receive`. New operation: advance the clock by `argument` hours. The
  model then has to allow `SessionError::Expired` from `send` once the
  age is 24 hours, and `Protocol(SessionExpired)` from `receive` once it
  is 24 hours plus the grace, on a direction that was not interfered
  with.
- New operations: `block_peer`, `remove_contact`, `stream_closed`.
- Operation codes: keep 0 to 13 as they are, so the committed seeds keep
  their meaning; change the selector from `% 14` to `% 22`; 14 and 15
  time, 16 and 17 block, 18 and 19 remove, 20 and 21 stream closed.

`fuzz/fuzz_targets/handshake_responder.rs`:

- It only admits with `PeerRecord::None`. Take the record from the two
  high bits of the first input byte (none, blocked, requested, accepted)
  and the piece size from the low six. Generalize the assertions after
  `admit`: first actions by record, `Deliver` only on a confirmed
  session, `MarkAccepted` only for a requested record.

Then:

- Update `crates/monolith-session/src/tests/seeds.rs`: the argument
  counts in `frame_seeds_are_complete_operation_lists`, new seeds for the
  new operations and records.
- Regenerate: `MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-session seeds`
- Update the tables in `fuzz/README.md` and `docs/TEST_PLAN.md` section 5.

### 6.2 Mutation testing

The two lists are scripts that were kept outside the repository and exist
only on the first development machine. The method: one exact-match
replacement in one source file, run the test filter that should fail,
restore the file, report caught or survived. A surviving fault in
authentication logic blocks the phase.

- Run the Phase 2 list again on the final commit. It had 78 faults in
  `transport.rs`, `card.rs`, the protocol `session.rs`, `resolver.rs`,
  `handshake.rs`, `key.rs` and the session `session.rs`. Several patterns
  moved with the review fixes and need adjusting.
- Add faults for the checks that the review fixes introduced:
  - the grace added to the age limit in `receive`;
  - the refusal in `set_transfer_active`;
  - `accept_sent` and `request_sent` in `may_send`;
  - the invitation comparison in `speaks_only_for_the_local_side`;
  - where the invitation comes from in both `admit` functions;
  - each call of `end()` that drops the keys;
  - the all-zero check in `TransportSecretKey::from_bytes`;
  - `OutboundPeer::admit` ignoring its record.
- Run the Phase 1 list again (28 faults in frames, text, cards, bodies,
  identity keys and session logic).

### 6.3 Full verification on the final commit

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo +1.85.1 check --workspace --all-targets --locked
    cargo +1.85.1 test --workspace --locked
    cargo deny check
    cargo audit
    cargo tree -d
    cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins --locked

Fuzz smoke run of all eleven targets, as `fuzz/README.md` describes, 25
seconds each. Phase 1 seeds must still match what the code produces
(`cargo test -p monolith-protocol --test fuzz_seeds`).

### 6.4 Documents

- `docs/RESOURCE_LIMITS.md` section 11.1: measure again. The session
  object gained a few fields in `e194dca`, so its size and the heap
  figures may have moved. The measuring program is not in the repository.
  It used a counting global allocator, 2000 handshakes in a release
  build, and timed each stage of the responder separately.
- Read `PROTOCOL.md`, `CRYPTOGRAPHY.md`, `THREAT_MODEL.md`,
  `SECURITY_INVARIANTS.md` and `TEST_PLAN.md` once more against the code
  as it is after the review fixes.
- `fuzz/README.md`: the status paragraph, after the smoke run.
- Remove this file.

### 6.5 Final report

The Phase 2 brief asks for a final report of 30 items and then a stop:
no Phase 3, no merge to `main`. The brief is not in the repository; the
owner has it. The report covers at least: the design chosen and why, the
library and provider, every Phase 1 wire change with its reason, the key
lifecycle, the test and fuzz inventory, the mutation results, the
dependency review with `cargo tree -d`, the measurements, the findings of
the two reviews and what was done about each, and what is not claimed.

## 7. Open points that are not Phase 2 work

Listed in `docs/DESIGN_QUESTIONS.md` section 3 and in the ADR:

- F-R2: the rules that bind the handshake to identities have no external
  review.
- F-R3: `snow` has one maintainer.
- F-R4, F-R5: no revocation of a transport key.
- F-R6, P2: a session is not bound to the onion address that was dialed.
- P8: no period in which two transport keys are answered.
- The budgets and rates of `RESOURCE_LIMITS.md` sections 5 and 6, and the
  store that keeps the newest card of a contact, come with the
  application core in a later phase.
