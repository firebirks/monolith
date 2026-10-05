# Status of the Phase 4 work

Written on 2026-10-05. This file describes work in progress on the branch
`phase-4-contact-store`. It is removed when Phase 4 is closed; it does not
belong on `main`.

Branch: `phase-4-contact-store`, created from `2f27dcb`, the commit on
which Phase 3 was verified. `main` holds four commits after it
(`86adac1` to `6eed860`) that only change documents: they record the
completion of Phase 3 and remove the Phase 3 status file. They are not on
this branch; whoever merges it reconciles this file and the status lines
of `README.md` and `docs/ARCHITECTURE.md` with them.

## 1. What Phase 4 is

One durable, transactional and authoritative contact subsystem: one
contact store per local identity that decides about every card, admits
every session and holds the credentials of every contact; the encrypted
vault that keeps it; the retirement of live sessions; message 3 behind
the admission of the store; invitations and several local identities
through the store; the budget for strangers; the supervisor that
publishes each identity's Onion Service again after Tor lost it; and the
rotation and dialing policy. No change of Noise XK or of the wire format.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not rewritten or moved, and the
  branch is not merged.
- Nothing is pushed unless the owner asks for it. No force push. No
  tags.
- No history is rewritten.
- Commits use the repository-local identity
  `firebirks <336470935+firebirks@users.noreply.github.com>` as author and
  committer; check `git config --local user.name` and `user.email` before
  committing. The remote is `git@github-firebirks:firebirks/monolith.git`.
- Everything in the repository is in English, plain ASCII.
- Monolith never starts, configures or restarts Tor, never falls back to
  a direct connection, never resolves an onion name locally, and has no
  generic control port API.
- The normative documents in `docs/` define the intended behavior and
  this file records what is implemented. A disagreement between them is a
  bug to resolve. A security or protocol question is decided by the
  owner, not in code.
- Phase 4 is not declared complete in any document before its final
  verification and the owner's decision.

## 2. Decisions taken

`docs/DESIGN_QUESTIONS.md` section 11.2, P4-1 to P4-15. In short:

- one evaluation of a card, whatever path it took, from one table; the
  conservative reading where Phase 3 had two (P4-1);
- message 3 made only in the admission of the responder, in the session
  crate (P4-2);
- one lock per contact, nothing awaited under a lock (P4-3);
- commit before use: revocations act at once, grants wait until durable;
  a failed write fails the installation closed (P4-4);
- one rule that withdraws every session that no longer stands for its
  contact, after every change (P4-5);
- the vault as `docs/STORAGE.md` section 3 specifies; one vault per
  installation (P4-6, ST7, MI-3);
- budget values for several identities (P4-7, MI-2) and eviction of the
  oldest silent stranger (P4-8);
- the publication supervisor (P4-9);
- the rotation and dialing policy (P4-10, P4-11, CR-1);
- `MAX_ACTIVE_INVITATIONS` stays 16; capabilities and revocations are
  durable (P4-12).

No wire change; Noise XK unchanged; the known-answer vectors of
`docs/PROTOCOL.md` 16.1 are unchanged.

## 3. What is done

Commits on the branch, oldest first:

| Commit | Content |
| --- | --- |
| `5292c98` | One decision about a card: `Credentials::relation` and `evaluate`, `contact::decide`, the exhaustive table test. |
| `c8d8733` | Message 3 made only by `OutboundPeer::admit`. |
| `8431c11` | `monolith-storage`: the vault, its records, the atomic replace, the directory on disk and in memory with its crash model. |
| `69ab17c`, `d68bba3` | `monolith-core`: the contact store, local identities and the installation, durability, requests and invitations, the budget for strangers, the dial plan; every link admitted through the store. |
| `ca41199`, `ba5b263` | The publication supervisor. |
| `06439ff` to `99b667a` | Tests: restart, a crash at every step, races, invitations (T-INV), several identities (T-MI), the contact budget, rotation; a fix found by fuzzing (`1897a71`); fuzz targets `vault_payload` and `contact_store`. |
| `b687c03`, `31b5a5e`, `404c4a1` | `dev-node`, a development node over a persistent vault, and two fixes found by the private network test. |
| `91ccaae`, `cea6d38`, `e78b0f8`, `b463500` | The private Tor network test extended to the contact store; Tor stopped reliably. |
| `5706c1d`, `b3bcaa2`, `12e801c` | Mutation faults carried over to the Phase 4 code, the Phase 4 list, CS44, and CS28 expected to survive. |
| `69687f6` | A source scan for the message 3 path and for global state. |
| `75b3a22` | A measurement of vault unlock time (`docs/STORAGE.md` 3.3). |
| `49ed51d` | The vault format against an independent reader and writer, and a pinned test vector. |
| `e35f7e9`, `3e5965b`, `c6bdd58`, `2c077e0` | Tests of what peers observe (T-ORACLE-6 and 7, T-CONFIRM-1, T-CONTACT-1 and 3, T-INJ), and of the gaps the targeted mutation run found. |
| `86c2b40` onwards, between the above | Documents. |

Where things are:

- `crates/monolith-protocol/src/credential.rs` and `contact.rs`.
- `crates/monolith-session/src/handshake.rs` (`OutboundPeer`,
  `Message3`).
- `crates/monolith-storage/src/`: `vault.rs`, `record.rs`, `dir.rs`.
- `crates/monolith-core/src/`: `contacts.rs`, `identity.rs`,
  `persist.rs`, `requests.rs`, `strangers.rs`, `dialplan.rs`,
  `supervisor.rs`, and the changed `link.rs` and `budget.rs`.
- `crates/monolith-cli/src/node.rs`.
- Tests: `crates/monolith-core/tests/` (`store.rs`, `credentials.rs`,
  `invitations.rs`, `identities.rs`, `supervisor.rs`, `link.rs`,
  `structure.rs`, `tor_network.rs`), the tests in
  `crates/monolith-storage/src/`, fuzz targets `vault_payload`,
  `contact_store` and the extended `credential_sequence`, mutation faults
  `CS1` to `CS43`, `tests/tor-network/two-node.sh` steps 4 to 11.

## 4. What was verified during development

None of this is the final verification; code changed after it.

- fmt, clippy with `-D warnings` and the tests of every crate, on Linux,
  after each code commit.
- Fuzz runs of 60 seconds of `credential_sequence`, `vault_payload` and
  `contact_store`; one finding, fixed in `1897a71`.
- A targeted mutation run of the 43 Phase 4 faults of then and the 15
  faults carried over, on `5706c1d`: 54 caught, 4 survived. Q30, CS12 and
  CS17 were gaps in the tests and have tests since (`c6bdd58`,
  `2c077e0`), checked against their faults; CS28 needs two threads and is
  expected to survive (`mutation/README.md`).
- Development runs of the private Tor network test in a Linux container
  (`tests/tor-network/Dockerfile`, Tor 0.4.9.13, Chutney `ae3a33c`). They
  found the two `dev-node` faults fixed in `31b5a5e` and `404c4a1`, and
  that Tor with `Sandbox 1` can ignore SIGTERM, which the test handles
  since `cea6d38`. The run on `cea6d38` passed every step.
- `cargo deny check`, `cargo audit` (82 crates) and `cargo tree -d` on
  2026-10-05: clean, one duplicate pair in test builds only.
- The KDF measurement on the development machine (`docs/STORAGE.md`
  section 3.3).

## 5. What is left, in order

### 5.1 Final verification on the final commit

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo +1.85.1 check --workspace --all-targets --locked
    cargo +1.85.1 test --workspace --locked
    cargo +stable test --workspace --locked
    cargo deny check
    cargo audit
    cargo tree -d
    cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins --locked

- Seeds and vectors: the `fuzz_seeds` tests of `monolith-protocol`,
  `monolith-tor`, `monolith-storage` and `monolith-core`, and the
  known-answer vectors in the session tests.
- Fuzz smoke run of all 17 targets, and longer runs of
  `credential_sequence`, `contact_store`, `vault_payload`,
  `session_frames`, the handshake targets, `tor_control_reply` and
  `socks_reply`.
- Mutation suite: `python3 mutation/run.py all 3`, with the expected
  survivors of `mutation/README.md`.
- `tests/network/fail-closed.sh` and the private Tor network test,
  `tests/tor-network/README.md`. The private network test is a hard
  gate.
- `git log 2f27dcb..HEAD --format='%an %ae %cn %ce' | sort -u` shows only
  the firebirks identity.

No file changes once that run has started; if one does, the run starts
again on the new commit. The results go to the final report; the owner
decides whether Phase 4 is complete.

### 5.2 Open points that are not Phase 4 work

`docs/DESIGN_QUESTIONS.md` sections 3 and 11.3: the session manager with
the duplicate rule and per-contact rates, the reconnect scheduler, the
outbound queue and the message store (ST1), the command and event
interface for a front end, the review of `MaxStreams` (C1), the KDF
measurement on the four targets of `docs/STORAGE.md` 3.3, a target port
per identity on Tails and Whonix (MI-1), and the phase that offers
several identities in an interface (MI-4).
