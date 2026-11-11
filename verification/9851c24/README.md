# Final verification of Phase 8: f7f8d81, 9851c24, then c90d5e5

This directory holds the outputs of the final verification of Phase 8
and the scripts that ran it. They are kept on the branch
`verification-9851c24`, which changes nothing outside this directory and
is not merged into `main`.

- Verified production: `9851c24`
  (`9851c24b9814092c2a50efdc6da51d99e0633460`) of
  `phase-8-file-transfer`. The candidate frozen first was `f7f8d81`
  (`f7f8d81d3f21c594e9d801a174beeeffaf39328c`), where the stages ran;
  `7d31573` brought the oracle of the fuzz target `ui_present` up to
  `PROTOCOL.md` section 9, with a seed and its seed test, and `9851c24`
  moved the pattern of the mutation fault FT53. Neither changes
  production code or a test other than the seed test; what they change
  ran again on the commit that changed it.
- Verified tests and tooling: `c90d5e5`
  (`c90d5e54735542764fe46855ad68cbfd342273dc`). `c896861` made the
  mutation runner keep the whole output of every run; `c90d5e5` changed
  the hostile-sender test of `tests/transfer_restarts.rs` and added
  `docs/DESIGN_QUESTIONS.md` 15.4. Production code, manifests and the
  lockfile are those of `9851c24`. Gate C ran on `c90d5e5`.
- Documentary closure: `c0e8a6d`
  (`c0e8a6deb12a6c2639c20c1a7b4a3b6a3440dbb8`), the tip of
  `phase-8-file-transfer`. It changes documents only.

The final report is section 4 of the status file of Phase 8
(`STATUS.md`, kept in the history of `phase-8-file-transfer`, removed on
`main`); the owner accepted it on 2026-11-11. `docs/DESIGN_QUESTIONS.md`
15.5 records the completion, and `mutation/README.md` the mutation
results. The findings of the independent reviews of FT-K and their fixes
are in `docs/DESIGN_QUESTIONS.md` 15.3.

The two packages are not here; their sizes and SHA-256 are below and the
`.sha256` files are kept. Nor are the test binaries kept during the
diagnosis of `tests/transfer_restarts.rs`, the fuzz corpora, or the
copies of the source; the SHA-256 of every test binary compared are in
the `*binaries*.json` files.

Local paths in the logs are written as `<repo>` (the checkout, clone or
copy of the commit), `<home>` (the home directory in the Linux VM),
`<winhome>` (the home directory on Windows), `<work>` (the working
directory of the verification, whose `verify8/` is `scripts/` here),
`<host>` (the name of the machine) and `<user>` (the local user in
listings). Box drawing is written in ASCII, other characters outside
ASCII as `<U+XXXX>`, and every text file has LF line ends.

## Environment

- One machine, Windows 11 with WSL2: a Debian VM, kernel
  6.18.33.2-microsoft-standard-WSL2, with 4 CPUs and 12 GB, 10 GB from
  2026-11-05 18:25, and Docker Desktop for the container.
- Windows: a detached worktree at the commit with a target directory of
  its own; Rust 1.95.0 and 1.85.1, and 1.99.1 for the run on `9851c24`.
- Container (image `monolith-verify:phase7`, whose Dockerfile is on the
  branch `verification-bbf1279`, `scripts/full-740b671/container/`): Rust
  1.95.0, 1.85.1, 1.99.0 and 1.99.1, Tor 0.4.9.13, Chutney at `ae3a33c`,
  strace 6.1. The commit was exported without `.git` (`git archive`); the
  peer of `phase-7-complete` was built from the tag (binary SHA-256
  `c0c6999bdd9326b20e955d415018205a09416f0c69982d3b363263bf01076913`).
- WSL: a clone at the commit, Rust 1.95.0, nightly 1.101.0 (2026-10-10)
  with cargo-fuzz 0.13.2, cargo-deny 0.20.2, cargo-audit 0.22.2. The
  mutation runs ran in it, two copies, two workers.

## Dates

- `f7f8d81`: frozen 2026-11-05 07:31; the container stage to 09:05, the
  Windows stage to 09:51, the Windows mutation lists to 10:57, the WSL
  stage to 12:05 (`full-f7f8d81/launch/master.log`).
- `7d31573`: 2026-11-05, 11:34 to 11:45 the fuzz checks of `ui_present`.
- `9851c24`: FT53 alone 14:14 to 14:39; the full mutation run 2026-11-05
  14:39, interrupted at 18:25 (FT1 to FT25 ended), resumed 2026-11-07
  00:08 (FT26 to FT79 to 05:54), stopped by the owner at 08:35 during
  the Phase 7 list; Rust 1.99.1 on Windows 2026-11-07 08:57 to 09:11;
  gate B 2026-11-08 23:12 to 2026-11-09 06:01; gate C 2026-11-09 13:58,
  stopped at 14:24 before any fault; its diagnosis to 2026-11-09 23:46.
- `c896861`: the stress of `tests/transfer_restarts.rs` 2026-11-10 to
  12:32; gate C from 12:51, stopped at 13:22 before any fault.
- `c90d5e5`: the check of the corrected test 2026-11-10 to 14:40; gate C
  from 14:46, faults from 15:17; Windows restarted the host for an update
  at 00:44 on 2026-11-11 with 96 of 98 ended; the copies restored, the
  controlled start again from 07:58, FT29 and HR77 to 09:39.

## Layout

- `full-f7f8d81/`
  - `windows/`: `summary.txt`, a log per step, the pictures of every
    fixture (`screens/`), of the workflows (`scenarios/`) and of the file
    scenarios (`files/`), the client photographing its own window, the
    idle client (`idle/`), the checksum of the Windows package.
  - `container/`: `summary.txt`, `environment.txt`, a log per step
    (`two-node.log`, `fail-closed.log`, `tree-d.log` among them), the
    pictures.
  - `wsl/`: `summary.txt`, `environment.txt`, `deny.log`, `audit.log`,
    `identity.log`, `trailers.log`, the fuzz runs (`fuzz-summary.txt`),
    the client on WSLg, `package/` (the listing and the checksum).
  - `mutation-windows/`: the lists `phase7w` and `phase8w`, their logs
    and results files.
  - `launch/`: the log of the chain that ran the stages, the container's
    launch, and the log of the watchdog of memory and disk.
- `recheck-7d31573/`: the Linux checks of the seed test, fmt and clippy
  (`linux.txt`), and the fuzz checks of `ui_present` (`wsl/`), with the
  failing input reproduced on `f7f8d81`.
- `stable-9851c24/windows/`: the workspace tests on Rust 1.99.1.
- `mutation/`
  - `final8-mutation.txt`: one line per run; `reconcile.txt`: every run
    against `mutation/faults.py`.
  - `phase8-9851c24/`: the Phase 8 list, part 1 (FT1 to FT25, log only:
    the runner writes its results file at the end) and part 2 (FT26 to
    FT79, log and results file).
  - `ft53-9851c24/`: FT53 alone, after its pattern was moved.
  - `phase7-part1-9851c24/`: the 23 faults of the Phase 7 list that ran
    before the owner stopped the full run (HR1 to HR11, HR17 to HR28).
    The line of `final8-mutation.txt` for that stop says 24 by a slip;
    the log holds 23.
  - `selection/`: the audit of the selection of 2026-11-09
    (`audit_selection.out`, `audit_tests.out`, `reconcile_ids.out`), the
    ids of gate C (`run96.txt`, `run98.txt`), and the changed bodies of
    old tests (`old_test_bodies.txt`).
  - `gate-b-9851c24/`: the runner log, the results file, the launch log
    and the proptest regression cases the faults left in the copies.
  - `gate-c-9851c24/`: the runner log of the stop, the unmodified runs
    that did not fail again (`diag-unmod/`, `diag-restarts/`), and the
    replay of the start of the gate in three cycles (`diag-gatec/`).
  - `transfer-restarts/`: the stress of the binary (`stress/`), the stop
    of gate C on `c896861` with the whole output (`gate-c-c896861/`), the
    SHA-256 of the test binaries of `9851c24` (`binaries-9851c24/`), and
    the check of the corrected test (`fixcheck/`: the patch, the
    deliberate violations `violate.py`, `progress.log`, `progress2.log`,
    `logs/`).
  - `gate-c-c90d5e5/`: the controlled start (`progress.log`, `diag-c3/`),
    the runner's own logs of every run (`runner-logs/`), the runner log
    (`final8-mutation-C3.log`, `gate-c3.log`), the restart of Windows
    (`reboot/`: the faults each copy held, the hashes of the logs kept,
    the restore), the resumed run (`resumed/`), the results of FT29 and
    HR77, the 98 results in one file (`results-C3-merged.json`: the 96
    rebuilt from the logs with the runner's own classification), and the
    check of each verdict (`validation.txt`).
- `scripts/`: the scripts that ran all of this; `selection-and-watch/`
  holds those of the selection, the property check and the watchdog.

## Results on f7f8d81, and 9851c24 where named

| Stage | Windows | Linux |
| --- | --- | --- |
| fmt, clippy, clippy with fixtures, `cargo doc` | pass | pass |
| tests with the fixtures, 1.95.0 | 84 test runs, 1458 passed, 0 failed, 3 ignored | 84 test runs, 1444 passed, 0 failed, 3 ignored |
| `check --all-targets`, 1.85.1 | pass | pass |
| tests, 1.85.1 | 1458 passed, 0 failed, 3 ignored | 1444 passed, 0 failed, 3 ignored |
| tests, 1.99.1 | on `9851c24`: 1458 passed, 0 failed, 3 ignored | 1444 passed, 0 failed, 3 ignored |
| `cargo tree -d` | - | pass |
| T-NET-1, `tests/network/fail-closed.sh` | - | pass |
| `tests/tor-network/two-node.sh` | - | pass, steps 1 to 32, 95 checks, 1887 s, 3 restart cycles |
| workflows by script, file scenarios | pass | pass |
| pictures of every fixture, frame measurements | pass | pass |
| `tor_smoke` on the in-memory Tor | pass | - |
| packaged client, idle | 94 ms of CPU over 10 s, working set 161 MB, private 137 MB | - |
| package | `monolith-desktop-0.0.0-windows-x86_64.zip`, 5,034,903 bytes, SHA-256 `27c95a5a748d85b2d7083badc7977b1db5089fd99ca7f4a0ce2db6baeb83d112` | `monolith-desktop-0.0.0-linux-x86_64.tar.gz`, 6,974,206 bytes, SHA-256 `f7efb83b2329e90acd38233c0d998a5213fb2f71410a4472190cc86a720864b0` |

The 14 more tests on Windows are those of code built on Windows only.
The ignored ones are two tests the private network test runs and the
measurement of the unlock time.

WSL: deny passed (advisories, bans, licenses, sources); audit passed
over the 445 crate dependencies of the lockfile. Fuzz: 22 targets for 25
seconds each (libFuzzer reports 26) and six for 600 seconds; 21 and five
without a crash. `ui_present` failed at once in both: its oracle had not
been updated for the marks of `082e1d6`, and the product was right. On
`7d31573` the failing input and the seed `controls.bin` pass, and
`ui_present` ran 25 and 600 seconds (42,849 and 942,468 executions)
without a crash.

## Mutation

The brief asked for one complete baseline of Phases 1 to 8. It was
started on `9851c24` and stopped on the owner's request; targeted gates
took the place of the rest, on the owner's rules for converging a
release. Executed runs and motivated exclusions are kept apart:

- Run on the candidate, all caught: FT1 to FT79 (`phase8-9851c24/`; FT29
  by the timeout alone); 23 of the Phase 7 list; gate B, 84 faults on
  `9851c24`; gate C, 98 faults on `c90d5e5` (96 new and MR26, FT29 again).
  Of the 626 faults of Phases 1 to 7 for Linux, 203 ran.
- Run on `f7f8d81`, all caught: the 14 faults of `phase7w` and FT80;
  `f7f8d81..9851c24` changes no code built on Windows only.
- Not run: the nine expected to survive (B3, B4, S15, S24, CAP2, CAP3,
  Q14, Q18, Q29; last verdicts on `bbf1279`, survived as expected), and
  414 whose killing test is in a file Phase 8 did not change and whose
  mutated function it did not edit.
- Exceptions the owner approved, not counted as runs: the runner change
  of `c896861`, and 38 faults the test change of `c90d5e5` selects only
  by its file.
- Caught by the timeout alone: MR26 (gates B and C), MR25 and FT29 (gate
  C); by the timeout and tests that failed: MR9, HR31 (gate B), Q1, CS30
  (gate C). K4 is caught by a doc-test the runner does not name.

`gate-c-c90d5e5/validation.txt` reads each verdict of gate C against the
whole output of its run and the earlier evidence; `reconcile.txt` counts
every run against the lists. The final report gives the reasons in full
(section 31) and the history of the two stops of gate C on the
hostile-sender test (section 39): an intermittent anomaly, attributed
with reasonable evidence to the dependence of the former test on the
scheduling of persistence; its exact cause was not reproduced
deterministically, and the test was replaced by a check with controlled
interleavings.

Known limit, kept: the lock of the disk has no order. A chat already
durable goes ahead of the transfer on the transport; no bound is
guaranteed on the time a chat takes to become durable while transfers
write (`docs/DESIGN_QUESTIONS.md` 15.4).

## Reproducing

The scripts are those that ran: `scripts/master.ps1` chained the stages
on `f7f8d81` (`scripts/container/`, `scripts/windows-stage.ps1`,
`scripts/wsl/final-wsl.sh`), `scripts/wsl/final-mutation.sh` and
`resume-mutation.sh` the full mutation run, `targeted-mutation.sh` gate
B, `targeted-c.sh`, `c3-unmodified.sh`, `targeted-c3.sh` and
`targeted-c3r.sh` gate C, `fixcheck/fixcheck.sh` (under
`mutation/transfer-restarts/`) the check of the corrected test. They
expect the paths written here as placeholders. The mutation runner is
`mutation/run.py` of the commit; `python3 mutation/run.py all 2 <ids>`
runs a list of faults by id, and `mutation/README.md` says how. The fuzz
targets run as `fuzz/README.md` says, from the committed seeds.
