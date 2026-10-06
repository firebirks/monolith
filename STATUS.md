# Status of the Phase 4 work

Written on 2026-10-05, when the owner stopped the final verification to
have findings of their own fixed, and brought up to date on 2026-10-06,
when those findings were closed, and again at the end of that day, in the
middle of a further round of review closure (section 5.1), so that the
work can be picked up on another machine. This file describes work in
progress on the branch
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
  tags. The owner asked for this branch to be pushed on 2026-10-05 and
  on 2026-10-06, each time to pick the work up on another machine.
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
| `d5f7721` to `a5383cb`, `6d26193` to `c42c698`, `9c5ce64` | The fixes of the two reviews of the Phase 4 code (`docs/DESIGN_QUESTIONS.md` 11.4): the write job, which also fails closed on its own and holds nothing of the installation once its outcome is out, the closing of a deleted identity, durable rotation steps, an end of rotation only after a durable switch, the snapshot as one cut, cancelled admissions forgotten, the flush of a new directory, the erasing encoder, and their mutation faults. |
| `e29d7f2`, `49a225a`, `d84eb80` | CS45 following the write job; deadlines instead of hangs in two tests, so that CS46 and CS50 are caught at once; the runner's note for a fault caught by its timeout alone. |
| `854fcf7` to `881ed27` | The closure of the owner's review (`docs/DESIGN_QUESTIONS.md` 11.5): thirteen findings reproduced and fixed, a second review of the whole Phase 4 code and reviews of its fixes, all fixed; CS28 caught and CS52 back by hooks; faults retargeted; the structure scan fixed. |
| `cc8650f` to `c8322e9` | A further round of review closure, not finished (section 5.1): three findings of an outside review and the mutation issues it raised, and what independent reviews of the whole code then found. |
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
  `observability.rs`, `structure.rs`, `tor_network.rs`), the unit tests of
  `persist.rs` and `contacts.rs`, the tests in `crates/monolith-storage/`
  (`src/`, `tests/vault_format.rs`, `tests/kdf_benchmark.rs`), fuzz
  targets `vault_payload`, `contact_store` and the extended
  `credential_sequence`, mutation faults `CS1` to `CS109`,
  `tests/tor-network/two-node.sh` steps 4 to 11.

## 4. What was verified

### 4.1 During development

None of this is the final verification; code changed after it.

- fmt, clippy with `-D warnings` and the tests of every crate, on Linux,
  after each code commit.
- Fuzz runs of 60 seconds of `credential_sequence`, `vault_payload` and
  `contact_store`; one finding, fixed in `1897a71`. Smoke runs of the 16
  targets (25 seconds each) on `42fa7e4` and on `2deef1c`, and runs of 10
  minutes of `tor_control_reply`, `socks_reply`, `handshake_initiator`,
  `handshake_responder` and `session_frames` on `42fa7e4`: no failure.
- A targeted mutation run of the 43 Phase 4 faults of then and the 15
  faults carried over, on `5706c1d`: 54 caught, 4 survived. Q30, CS12 and
  CS17 were gaps in the tests and have tests since (`c6bdd58`,
  `2c077e0`); CS28 needed two threads and was expected to survive then
  (`mutation/README.md`). A targeted run of every new or changed fault
  (CS12, CS15, CS17, CS28, CS30, CS44 to CS51, CS53 to CS55, Q30) on
  `e29d7f2`: 16 caught, CS28 survived as expected, exit code 0.
- Runs of the private Tor network test in a Linux container
  (`tests/tor-network/Dockerfile`, Tor 0.4.9.13, Chutney `ae3a33c`). They
  found the two `dev-node` faults fixed in `31b5a5e` and `404c4a1`, and
  that Tor with `Sandbox 1` can ignore SIGTERM, which the test handles
  since `cea6d38`. The runs on `cea6d38` and on `2deef1c` passed every
  step.
- The KDF measurement on the development machine (`docs/STORAGE.md`
  section 3.3).
- An independent review of the Phase 4 code: no path gives a contact
  session or message 3 to a key or a record that does not stand; eight
  findings on durability, cancellation and the life of an identity, all
  fixed with tests and mutation faults. A second review of the fixes
  found two paths they had left open and two faults that did not work;
  the mutation runner's baseline then found that the write job held the
  installation too long. All fixed (`docs/DESIGN_QUESTIONS.md` section
  11.4).
- The closure of the owner's review, on top of `03a626c`: thirteen
  findings, each reproduced by a test that failed before its fix, and the
  findings of a second review of the whole Phase 4 code and of three
  reviews of the fixes; recorded with commits, tests and faults in
  `docs/DESIGN_QUESTIONS.md` section 11.5. Every new or retargeted fault
  was applied by hand and seen caught by the tests named there.

### 4.2 Final verification, stopped after its first stage

The final verification was started on `8b6a6b8` and stopped by the owner
after its first stage, to fix findings of their own. The first stage
passed on `8b6a6b8`, with a clean tree:

| Check | Result |
| --- | --- |
| `cargo fmt --all --check` | clean |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` (1.95.0) | clean |
| `cargo test --workspace --locked` (1.95.0) | 33 test binaries, 619 tests passed, 2 ignored (the KDF measurement and the Tor test that the network script runs) |
| `cargo +1.85.1 check --workspace --all-targets --locked` | clean |
| `cargo +1.85.1 test --workspace --locked` | 619 passed |
| `cargo +stable test --workspace --locked` | passed; stable was 1.95.0 that day |
| `cargo deny check` | advisories, bans, licences, sources ok |
| `cargo audit` | no advisory; 82 crates |
| `cargo tree -d` | `rand_core` 0.9.5 (tests only) and 0.10.1, nothing else |
| `cd fuzz && RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins --locked` | clean |
| `tests/network/fail-closed.sh` (T-NET-1) | passed |
| Identities of `2f27dcb..8b6a6b8` | 80 commits, all `firebirks <336470935+firebirks@users.noreply.github.com>` as author and committer |

Not run on `8b6a6b8`: the fuzz runs, the private Tor network test and the
full mutation run. Since the owner's fixes change the code, the whole
final verification starts again on the commit that ends them.

## 5. What is left, in order

### 5.1 The review closure in progress

The owner's first findings are closed; `docs/DESIGN_QUESTIONS.md`
section 11.5 records them and what the reviews of their fixes found. On
`642a5ed` the first stage of the final verification, the fuzz runs and
the private Tor network test passed; the code has changed since, so none
of that counts for the final commit.

The owner then brought three findings of an outside review of `642a5ed`
and two mutation issues. They are being closed now. The pattern behind
them is a logical name taken for an object instance: the same local
identity in another lifecycle, the same remote contact in another
record, the same counter in another identity, the same rotation context
in another rotation. Done, each with a test that fails on the code before
it and a mutation fault, in commits of their own:

| Item | What was wrong | Fix | Commit | Test, fault |
| --- | --- | --- | --- | --- |
| Outside review 1 | Handles kept after the last `Installation` handle was dropped still had authority: party, Onion Service secret, invitation cards, dial plans, changes made in memory; kept links still sent. | Dropping the installation closes its identities as a deletion does; a write under way ends on its own. `is_deleted` is `is_closed` now. | `cc8650f` | `tests/identities.rs`, `tests/supervisor.rs`, CS95 |
| Outside review 2 | `InvitationId` was a bare counter: an id of A worked at B. | Ids carry the identity instance, a random number drawn when the instance is made; so do rotation ids. | `b3a7cb5` | T-MI-11, CS96, CS87 |
| Outside review 3 | A late announcement marked a contact made again for the same identity. | `mark_announced` takes the `Announcement`, bound to its rotation, its contact instance and its session. | `8c0aff1` | `tests/credentials.rs` (six cases), CS97, CS98, CS77 |
| Outside review 4 | The test of CS51 missed the fault in one order of the map in 1024. | It checks at every read that the snapshot holds the lock of the map. | `f39f906` | `contacts::tests`, CS51 |
| Outside review 5 | CS47 and CS48 survived the whole core suite: other guards held. | Retargeted at the state they name: a change of a deleted identity made in memory, a deleted identity's Onion Service secret. | `e6b8e18` | `tests/identities.rs`, CS47, CS48 |
| Sibling | A request was answered by its sender: an answer for one request accepted a later one. | Requests have handles (`RequestId`). | `1314b9d` | `tests/invitations.rs`, CS99, CS67 |
| Independent review | `delete_identity` deleted whatever identity held the keys named. | It takes the instance. | `fc07b65` | `tests/identities.rs`, CS100 |
| Independent review | A forced switch or end decided for one rotation hit the next. | Both take the `RotationId`. | `c73482e` | `tests/credentials.rs`, CS101, CS102 |
| Independent review | `import` and `block` took out a request queued after them. | The queue is locked across the change. (`91a3415` was committed with a test failing; `780c3dc` adapts the test.) | `91a3415`, `780c3dc` | `identity::tests`, CS103, CS104, CS14, CS66 |
| Independent review | A card was returned for a capability revoked as it was created. | The card is made from the active set after the commit. | `3a6a823` | `identity::tests`, CS105, CS84 |
| Independent review | A `DialPlan` or the answering party handed out a party with its key. | Parties stay in the core; `DialPlan::local_card` instead. | `4c8e14b` | `tests/structure.rs`, CS107 |
| Independent review | A closed identity showed the card of a key it no longer answered with. | It keeps the party it answered with. | `85d6a33` | `tests/identities.rs`, CS106 |
| Independent review | A card of someone no longer a contact was dialed, up to message 2. | `dial` refuses before a stream. | `9f91b1f` | `tests/credentials.rs`, CS108 |
| Independent review | A contact request was made with the card of the moment, refused while switched. | It is made with the card and capability of its session (`AuthenticatedSession::invitation`). | `069ad2c`, `ffb81cd` | `tests/credentials.rs`, CS109 |
| Independent review | The dev node answered a request by sender, took ambiguous prefixes, and kept non-contact sessions as a peer's. | Fixed in `dev-node`. No automated test; the private Tor network test runs these commands. | `c8322e9` | none |

The mutation manifest at `c8322e9`: 324 faults, 109 of Phase 4 (CS1 to
CS109).

Left of this round, in order:

1. CS81 survives since `90b9cbe`: `consider` checks the closing itself,
   so removing the check in `apply` changes nothing seen. Retarget it to
   remove both, or drop it.
2. CS38 came out as a build failure, not a kill, in a run of every
   Phase 4 fault on `e6b8e18`; check it by hand. In that run CS6 and
   CS45 were caught with one test that also hung, CS17 and CS30 by the
   timeout as known, all others by failing tests.
3. A run of every Phase 4 fault on the final commit, recording for each
   the tests that kill it (the script of that run was in the session's
   scratch space and is to be written again, or `mutation/run.py`
   extended), and from it, for every Phase 4 fault: the invariant, the
   change, the test expected to kill it and why it fails. Repeat the
   new hook tests a number of times as a check.
4. Documents: `mutation/README.md` (counts 324 and 109, the new faults),
   `docs/DESIGN_QUESTIONS.md` section 11.5 (the rows of this round),
   `docs/SECURITY_INVARIANTS.md` and `docs/ARCHITECTURE.md` (instance
   handles: logical identity, lifecycle instance, durable record,
   runtime handle; `RequestId`, `DialPlan::local_card`, parties kept in
   the core, the dial refusal, `delete_identity` by instance), the
   public API changes in `docs/` wherever they are named.
5. fmt, clippy, all workspace tests, the fuzz smoke run of all 16
   targets, `tests/network/fail-closed.sh`, and the private Tor network
   test (the dev node changed).
6. Independent reviews of the round's fixes, for the three patterns:
   a handle that survives its parent's closure, an identifier that
   addresses another instance, a completion that reaches a replacement.
   Fix what is real.
7. The report of the round, of 23 items, ending with `READY TO FREEZE
   FOR PRE-FINAL CHECKS` or `NOT READY TO FREEZE`; then the final
   verification of section 5.2 on the frozen commit, when the owner
   says so.

### 5.2 Final verification on the final commit

Run in three stages, so that no check runs under a load that changes its
result; above all the mutation run must run alone, because a fault whose
tests time out under load would count as caught.

Stage 1, in order:

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
    tests/network/fail-closed.sh
    git log 2f27dcb..HEAD --format='%an <%ae> | %cn <%ce>' | sort | uniq -c

Stage 2, side by side:

- Fuzz smoke run of all 16 targets, 25 seconds each, from `fuzz/`:

      cargo +nightly fuzz run <target> corpus/<target> seeds/<target> -- \
          -malloc_limit_mb=64 -timeout=5 -max_total_time=25

  and runs of 10 minutes (`-max_total_time=600`) of
  `credential_sequence`, `contact_store`, `vault_payload`,
  `session_frames`, `handshake_initiator`, `handshake_responder`,
  `tor_control_reply` and `socks_reply`.
- The private Tor network test, a hard gate, as
  `tests/tor-network/README.md` says: build `monolith-cli` and the
  `tor_network` test of `monolith-core`, build the image of
  `tests/tor-network/Dockerfile`, and run `two-node.sh` in it with
  `MONOLITH_BIN` and `MONOLITH_LIFECYCLE_BIN` pointing at the two
  binaries. It takes about twenty minutes.

Stage 3, alone:

- `python3 mutation/run.py all 4` (four workers on a machine with twelve
  threads; fewer on a smaller one). Expected, at `c8322e9`: 324
  faults, 315 caught, 9 survivors, exactly those of
  `mutation/README.md` (B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29),
  exit code 0, once CS81 and CS38 are settled; the numbers change with
  the faults the round still adds. CS17 and CS30 are caught
  by the runner's timeout of 20 minutes each. The run takes several
  hours. The runner works on copies of `HEAD` under `mutation/work/`, so
  everything has to be committed first; its results go to
  `mutation/results-all-<commit>.json` (ignored by git).

No file changes once the verification has started; if one does, it
starts again on the new commit. The results go to the final report; the
owner decides whether Phase 4 is complete.

### 5.3 Final report

The brief of Phase 4 asks for a final report of 26 items and then a
stop, without a push, a merge or a tag; the brief is not in the
repository, the owner has it. Besides its items, the report states the
results of section 5.2 with the commit they ran on, and that `main`
holds four document commits after `2f27dcb` (see the top of this file).

### 5.4 What a machine needs

Rust 1.95.0 (pinned in `rust-toolchain.toml`), 1.85.1, stable, and
nightly with `cargo-fuzz`; `cargo-deny`, `cargo-audit`; Python 3;
`strace` and unprivileged network namespaces (`unshare -rn`) for
`tests/network/fail-closed.sh`; Docker for the private Tor network test
(the image installs Tor 0.4.9.13 from the Tor Project's repository and
Chutney at `ae3a33c`). Some tests take a while in a debug build: the
store tests about 45 seconds, the snapshot test of `contacts.rs` up to
50.

### 5.5 Open points that are not Phase 4 work

`docs/DESIGN_QUESTIONS.md` sections 3 and 11.3: the session manager with
the duplicate rule and per-contact rates, the reconnect scheduler, the
outbound queue and the message store (ST1), the command and event
interface for a front end, the review of `MaxStreams` (C1), the KDF
measurement on the four targets of `docs/STORAGE.md` 3.3, a target port
per identity on Tails and Whonix (MI-1), and the phase that offers
several identities in an interface (MI-4).
