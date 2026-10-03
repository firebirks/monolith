# Status of the Phase 3 work

Written on 2026-10-02 so that the work can be picked up on another machine.
This file describes the work on the branch `phase-3-system-tor`. Phase 3
is complete: the owner declared it so on 2026-10-03, on the verified
commit `2f27dcb`, tagged `phase-3-complete` (section 6). Whether this file
stays, or is removed before the branch goes to `main`, is the owner's
decision.

Branch: `phase-3-system-tor`, on top of `main` at `5ac8bdf`.
Last commit of work before this file: `ea8e64e`. Updated with the
credential binding review of the same day, the integration hardening of
2026-10-02 and 2026-10-03, and the final hardening of 2026-10-03
(sections 2, 3, 4, 6 and 7).

## 1. What Phase 3 is

The integration with a Tor the system runs: a SOCKS5 client for outbound
streams to Onion Services, a minimal control client for status and for
publishing an ephemeral or persistent v3 Onion Service, Tor streams under
the Phase 2 sessions, connection budgets, and the tests that go with them.
No storage, no contact store, no user interface. Phase 4 is not started.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not rewritten or moved, and the
  branch is not merged.
- Nothing is pushed unless the owner asks for it. No force push.
- No history is rewritten.
- Commits use the repository-local identity
  `firebirks <336470935+firebirks@users.noreply.github.com>` as author and
  committer; check `git config --local user.name` and `user.email` before
  committing. The remote is `git@github-firebirks:firebirks/monolith.git`.
- Everything in the repository is in English, plain ASCII.
- Monolith never starts, configures or restarts Tor, never falls back to
  a direct connection, never resolves an onion name locally, and has no
  generic control port API.
- The specification comes first. The normative documents in `docs/`
  define the intended behavior and this file records what is implemented.
  A disagreement between code and a normative document is a bug to
  resolve; neither stale text nor an accident of the code silently
  overrides the other. A security or protocol question is decided by the
  owner, not in code.

## 2. Decisions taken

- The six points of the Phase 3 design review, T3-1 to T3-6:
  `docs/DESIGN_QUESTIONS.md` section 5.1. In short: the Tails checks gate
  Phase 6, not the generic backend; two `ADD_ONION` forms, no
  `DiscardPK`; `status/bootstrap-phase` reduced to a progress number; one
  random SOCKS isolation token per contact in the `<torS0X>0` format;
  `PoWDefensesEnabled=1` requested, never claimed active;
  `MaxStreams=8` with `MaxStreamsCloseCircuit`, provisional.
- Contact cards and invitation capabilities, including P10 and P11:
  `docs/DESIGN_QUESTIONS.md` section 6, `docs/PROTOCOL.md` sections 12.2
  and 12.3. No wire change.
- The credential binding review, reopened by the owner: F2 gets a
  local-party half, F4 becomes successor credentials (active, authorized
  successor, pending successor, retired), admission reads the contact
  state after the handshake, sessions of a retired key are withdrawn, the
  duplicate rule looks at credentials first, rotation keeps the old key
  until the successor is promoted, and the per-contact endpoint claim is
  withdrawn. `docs/DESIGN_QUESTIONS.md` section 8. No wire change; Noise
  XK unchanged.
- The integration hardening, after two further static reviews: message
  3 only to a peer that may learn the local identity, read from the
  session and raced against the withdrawal; an older card of the active
  key is the contact; the deadlines of a session (frame, idle, age, the
  two unknown-session limits, a Close that is due) enforced while bytes
  are awaited; one end for every failure of a link, which gives back its
  slot; no outbound session for anyone but a contact; fuzz targets that
  check invariants instead of a transcript. `docs/DESIGN_QUESTIONS.md`
  section 9. No wire change; Noise XK unchanged.
- The final hardening, accepted residuals included: a link ends once, the
  handshake targets refuse a corrupted genuine message again, F4 is stated
  as implemented in the cryptography documents, a refused dial keeps its
  admission for the local side, and the private network test runs.
  `docs/DESIGN_QUESTIONS.md` section 10. No wire change; Noise XK
  unchanged.
- Several local identities as an architectural requirement:
  `docs/DESIGN_QUESTIONS.md` section 7, `docs/ARCHITECTURE.md` section
  1.1, invariants S39 to S46. No wire change, nothing implemented as a
  feature.

## 3. What is done

Commits on the branch, oldest first:

| Commit | Content |
| --- | --- |
| `b95d8e7`, `fb42683` | Phase 3 design review and the owner's decisions. |
| `0238a66` | Strict ServiceID codec (`monolith-identity/src/onion.rs`). |
| `daf35fc` to `13c2e3d` | Tor adapter: types and secrets, endpoint configuration, SOCKS5 client, control client with SAFECOOKIE, publication through the system Tor. |
| `1feb090` | rustfmt over the new code. |
| `43e550e`, `76d603c` | Backend tests against hostile scripted servers; `MockTorBackend`. |
| `4a4e4b5` | `monolith-core`: `link` (Tor streams under sessions) and `budget`. |
| `6c94ed9` | CLI: `tor status`, `doctor`, `dev-chat`. |
| `70c6770`, `a11539a`, `0a4478b` | Lints: no local name resolution, no unbounded channels, no tokio networking in the CLI. |
| `8177204`, `7d0fc80` | Fail-closed network test; private Tor network test (written, not run). |
| `e740da8` | Fuzz targets `tor_control_reply` and `socks_reply`. |
| `6a35b0c`, `531645e` | More tests: control loss before accept, a silent peer. |
| `e414598` | Phase 3 documents. |
| `024fbcd` to `13a9bf9` | Fixes from the two security reviews (section 5). |
| `c7bf65e`, `a59895d`, `ea8e64e` | Mutation suite extended to Phase 3; a lock against two runs; a timed-out or stopped fault ends its test binaries. |
| `98a1c7a` to `fa87ba2` | Contact card and capability semantics, with one card test. |
| `950d8ea` to `90ac497` | Several local identities: documents, comments, structural tests. |
| `3bc40c5` to `c7faad2` | The credential binding review: verification of the reported issues, the local party, credential states, fresh admission, withdrawal, the duplicate rule, mutation faults, documents, the fixes of the two review passes (`docs/DESIGN_QUESTIONS.md` 8.4 lists the commits). |
| `8f52eca` to `6bd9074` | The integration hardening: verification of the reported issues, the message 3 gate, session deadlines, the end of a link, fuzz invariants, documents, the fixes of two focused review passes, mutation faults (`docs/DESIGN_QUESTIONS.md` 9.4 lists the commits). |
| `94f5762` onwards | The final hardening: README, a link that ends once, the corruption assertion of the handshake targets, F4 in the cryptography documents, refused dials that keep their admission, the private network test brought to the current Chutney and extended (`docs/DESIGN_QUESTIONS.md` section 10). |

Note for the final report: `a11539a` also contains the Phase 3 list of
`mutation/faults.py`, which was staged by mistake. It was not rewritten.

Where things are:

- `crates/monolith-tor/src/`: `lib.rs` (the `TorBackend` and
  `OnionService` traits), `config.rs`, `secret.rs` (onion secret,
  isolation groups), `socks.rs`, `control/` (`reply.rs`, `command.rs`,
  `auth.rs`, `mod.rs`), `system.rs`, `stream.rs`, `mock.rs`, `status.rs`,
  `fuzzing.rs`.
- `crates/monolith-core/src/link.rs` and `budget.rs`.
- `crates/monolith-cli/src/main.rs` and `dev.rs`.
- Tests: `crates/monolith-tor/tests/system.rs`,
  `crates/monolith-core/tests/link.rs`, `tests/network/`,
  `tests/tor-network/`, fuzz targets in `fuzz/fuzz_targets/`, mutation
  suite in `mutation/`.

## 4. What was verified, and on which commit

None of this is the final verification: code changed after all of it.
Section 6.1 has to be run on the final commit. The credential review was
checked on Windows with fmt, clippy, the tests of every crate but
`monolith-tor`, a check on Rust 1.85.1, the fuzz build, the seeds of the
three changed fuzz targets, 60 seconds of the new `credential_sequence`
target, and two targeted runs of the changed and new mutation faults
without the `monolith-tor` tests, all caught. The 8 `monolith-tor` system
tests that fail on Windows fail the same way on `fe5327c`, before the
review. None of this replaces section 6.1.

The integration hardening was checked the same way on Windows, on
`46c5901`, its last code commit: fmt, clippy, the tests of every crate but
`monolith-tor`, a check on Rust 1.85.1, the fuzz build, the seeds of the
four changed fuzz targets, 120 seconds of `session_frames` and 60 of
`credential_sequence`, and targeted runs of the new, moved and affected
mutation faults without the `monolith-tor` tests: 74 faults, all caught
but S15 and S24, which are expected to survive. Not the final
verification either.

The definitive results are those of section 6.0, on `2f27dcb`. The
final hardening was checked during development on Windows (fmt,
clippy, the tests of every crate but `monolith-tor`, the seeds of the
handshake targets, and a targeted run of the 20 mutation faults in
`link.rs`, all caught) and in a Linux container (the tests of every
crate, the fail-closed test and the private network test, all passing).
The final verification is sections 6.1 and 6.2, on the final commit.

| Check | Result | Run on |
| --- | --- | --- |
| `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` | clean | `90ac497` |
| Tests of `monolith-core` and `monolith-tor` | pass | `90ac497` |
| Tests of `monolith-protocol` and `monolith-session` | pass | `b99f0b1`; no code of theirs changed since |
| Fail-closed network test (T-NET-1) | pass | before `98a1c7a` |
| `cargo deny check`, `cargo audit` | clean, 71 crates | before `98a1c7a` |
| `cargo tree -d` | one pair: `rand_core` 0.9.5 (tests only) and 0.10.1 | before `98a1c7a` |
| Mutation suite, all phases, 177 faults | stopped at 82; only expected survivors up to there | `a59895d`, intermediate |
| Fuzz smoke run of the 14 targets | not run in Phase 3 | |
| Private Tor network test (T3-7) | pass, in a Linux container with Tor 0.4.9.13 and Chutney `ae3a33c` | `2f27dcb`, final |
| Measurements of `RESOURCE_LIMITS.md` 11.2 | measured | before `98a1c7a` |

The intermediate mutation run was stopped because the final run has to
be made on the final commit. While stopping it, a test binary left
running by an earlier, discarded run was found; it had slowed the
intermediate run down. `ea8e64e` makes the runner end the test binaries
of a fault on a timeout and when it is stopped. An earlier full run had
been discarded because two runners shared the same copies; `a59895d`
prevents that.

The measuring program of section 11.2 is not in the repository.

## 5. The two review passes

Two independent security reviews were made of the Phase 3 code. Fixed:

- SAFECOOKIE: the cookie file is pinned in the configuration; an endpoint
  whose `PROTOCOLINFO` names another file is refused before
  `AUTHCHALLENGE`; the cookie must be a regular 32-byte file that its
  group and others cannot write, checked before it is opened.
- `Endpoint` is opaque; only its validating constructors make one, and
  the connection code checks the rule again.
- Lint gaps: tokio networking kept out of the CLI, unbounded channels
  banned.
- The dial and unknown-session budgets are enforced.
- A listener error no longer ends the accept loop.
- A failed write marks the session stream as closed.
- The intermediate copies of a key in base64 are erased.
- The trust placed in the control endpoint is documented
  (`docs/THREAT_MODEL.md` adversary O).

## 6. What is left, in order

### 6.0 Result

The final verification ran on `2f27dcb99df0f86bb1c539ff69b2323a43d75a05`,
with no file changed while it ran, and passed;
`docs/DESIGN_QUESTIONS.md` section 10.4 records it. The owner declared
Phase 3 complete on that commit, accepted the residuals A, B and C of
section 10, and had the branch pushed at that commit. The commit is
tagged `phase-3-complete`. Phase 4 has not started.

What follows is the procedure that was run.

### 6.1 Final verification on the final commit

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

- Mutation suite: `python3 mutation/run.py all 3`. Expected: 218 faults,
  exit code 0, survivors only B3, B4, S15, S24, CAP2, CAP3, Q14, Q18 and
  Q29 (`mutation/README.md`). It was run with two workers, for memory, and
  took about four hours.
- Fuzz smoke run of all 14 targets, 25 seconds each, as `fuzz/README.md`
  describes, and longer runs of the Tor parsers, the session and
  credential targets and the handshake targets.
- Seeds and vectors: `cargo test -p monolith-protocol --test fuzz_seeds`,
  `cargo test -p monolith-tor --test fuzz_seeds`, and the known-answer
  vectors of `PROTOCOL.md` 16.1 in the session tests.
- `tests/network/fail-closed.sh` (needs `strace` and `unshare`).
- `git log main..HEAD --format='%an %ae %cn %ce' | sort -u` shows only the
  firebirks identity.

Tools needed: Rust 1.95.0 (pinned in `rust-toolchain.toml`), 1.85.1,
stable, nightly with `cargo-fuzz`, `cargo-deny`, `cargo-audit`, Python 3,
`strace`. The tests of `monolith-tor` need a Unix system: 8 of its system
tests fail on Windows. The Linux part ran in a container with Tor
0.4.9.13 from the Tor Project's repository and Chutney `ae3a33c`; the
fuzz targets ran on the Windows host.

### 6.2 Private Tor network test

`tests/tor-network/two-node.sh` with Chutney and `tor` and `tor-gencert`
0.4.9.5 or later (`tests/tor-network/README.md`). It is a hard gate of
Phase 3: if it cannot be run on the final commit, Phase 3 is not
complete, and the report names the blocker.

### 6.3 Final report

The final report of 39 items was given to the owner. Its point 38, that
nothing was pushed, does not apply: the owner asked for the push during
the run. The notes this file kept for that report, the mutation list that
landed in `a11539a` and the discarded mutation runs with the runner fixes
of section 4, are in `docs/DESIGN_QUESTIONS.md` section 10.4 as well.

## 7. Open points that are not Phase 3 work

Listed in `docs/DESIGN_QUESTIONS.md` section 3:

- C1 to C3: `MaxStreams`, proof-of-work queue parameters, confirming
  reachability without `HS_DESC`.
- MI-1 to MI-4: several local identities (a target port per identity on
  Tails and Whonix, budget values, storage modes, the phase that offers
  them).
- The value of `MAX_ACTIVE_INVITATIONS`, the active set, revocation and
  the tests T-INV and T-MI come with the contact store of Phase 4.
- One evaluation of a card for import and admission, in the contact store
  of Phase 4 (`docs/ARCHITECTURE.md` section 1.2,
  `docs/DESIGN_QUESTIONS.md` 10.1). Until then `import` holds a card older
  than an announced successor as pending and admission calls it stale.
