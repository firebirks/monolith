# Final verification of 68ac655

This directory holds the outputs of the final verification of Phase 6 on
the frozen commit `68ac655` (`68ac655bb82f4d423d46581bdc9a1816f8f87161`)
of `phase-6-desktop-client`, and the scripts that ran it. They are kept
on the branch `verification-68ac655`, which changes nothing outside this
directory and is not merged into `main`. The final report is section 6
of the status file of Phase 6 (`STATUS.md` at `6180f06`); the owner
accepted it on 2026-10-18. `docs/DESIGN_QUESTIONS.md` 13.3 records the
completion.

The two packages are not here; their sizes and SHA-256 are below and in
item 22 of the report, and the `.sha256` files are kept.

Local paths in the logs are written as `<repo>` (the checkout, clone or
copy of the commit), `<home>` (the home directory in the Linux VM),
`<winhome>` (the home directory on Windows), `<work>` (the directory of
the scripts) and `<host>` (the name of the machine). The box drawing of
`cargo tree` is written in ASCII.

## Environment

- One machine, Windows 11 with WSL2: a Debian 13.4 VM with 4 CPUs and
  12 GB, kernel 6.18.33.2-microsoft-standard-WSL2, and Docker Desktop for
  the container.
- Windows: a detached worktree at the commit with a target directory of
  its own; Rust 1.95.0 and 1.85.1.
- Container (`scripts/Dockerfile`, the image built for Phase 5):
  `rust:1.95-bookworm` (Debian 12), Rust 1.95.0, 1.85.1 and stable
  1.99.0, Tor 0.4.9.13 from the Tor Project's repository, Chutney at
  `ae3a33c`, strace 6.1. The commit was exported without `.git`
  (`git archive`) and marked with its hash.
- WSL: a clone at the commit, Rust 1.95.0, nightly 1.101.0 (2026-10-10)
  with cargo-fuzz 0.13.2, cargo-deny 0.20.2, cargo-audit 0.22.2. The
  Linux package was built in this clone, since `packaging/linux/build.sh`
  takes the root of the repository and the commit time from git. The
  mutation runner ran in a second clone at the commit, with two workers.

## Dates

All on 2026-10-15 unless said otherwise. The commit was made at 06:19
and frozen at 06:44.

- Windows stage: 06:44 to 07:15.
- WSL stage: 06:44 to 07:18.
- Container stage: 11:41 to 12:27 (the run that counts; below).
- Linux package: 12:35, in the WSL clone.
- Mutation lists: 07:58 to 2026-10-17 20:41, with four interruptions
  (below).
- T6 run again alone: 2026-10-18, 00:14 to 00:44.

## Layout

- `windows/`: `summary.txt` (a line per step), a log per step, the
  pictures of every fixture (`screens/`: 68 fixtures, light and dark, at
  1180 by 760, 820 by 600 and 760 by 520), the workflows A to G
  (`scenarios/`), the release client with the `screenshot` feature
  photographing its own window (`gui-light.png`, `gui-dark.png`), the
  settings files the client wrote (`idle/`, `gui-*/`) and the checksum of
  the Windows package.
- `container/`: `summary.txt`, `environment.txt`, a log per step, the
  pictures of every fixture and of the workflows, as on Windows.
- `wsl/`: `summary.txt`, `environment.txt`, `deny.log`, `audit.log`,
  `identity.log`, `trailers.log`, the fuzz build and every fuzz run
  (`fuzz-summary.txt`), the client photographing its own window on WSLg,
  and `package/`: the log and the checksum of the Linux package.
- `mutation/`: `final6-mutation.txt` (a line per list or part of a list),
  the log of every part (`...-resumed.log` for the part after an
  interruption, `...-phase5-interrupted.log` for the first run of the
  Phase 5 list), the results files, `outcomes.txt` (each id with exactly
  one outcome, from `scripts/outcomes.py`), and `pause/`: what the last
  interruption recorded (when, which list, the faults left).
- `alone/`: T6 run again alone.
- `before-freeze/`: UI146 to UI160 run on `68ac655` itself, from 06:19 to
  06:43, before the freeze.
- `launch/`: `final.log` (the export, the containers and Docker Desktop),
  `launch-wsl.log` (each run started in WSL, the targeted runs of the
  Phase 6 list before the freeze among them) and `final6-chain.log` (the
  clones, the WSL stage and the start of the mutation lists).
- `scripts/`: the scripts that ran all of this.

## Results

Windows stage, all passed (`windows/summary.txt`):

| Step | Result | Time |
| --- | --- | --- |
| `cargo fmt --all -- --check` | pass | 2 s |
| clippy, all targets, warnings denied | pass | 26 s |
| clippy, all targets, with `monolith-ui/fixtures` | pass | 3 s |
| tests, Rust 1.95.0, with the fixtures | 76 test runs, 1165 passed, 0 failed, 2 ignored | 545 s |
| `check --all-targets`, Rust 1.85.1 | pass | 27 s |
| tests, Rust 1.85.1, with the fixtures | 76 test runs, 1165 passed, 0 failed, 2 ignored | 581 s |
| Windows package | `monolith-desktop-0.0.0-windows-x86_64.zip`, 4,547,125 bytes, SHA-256 `a4b5aa04f627dbff2d47f0266170f10c235927cc453085fb59583dcedda56e6f` | 78 s |
| workflows A to G by script | pass, 17 pictures | 131 s |
| `tor_smoke` on the in-memory Tor | pass | 9 s |
| pictures of every fixture | 408 | 463 s |
| frame measurements | at most 1.17 ms a frame after the first | 18 s |
| the packaged client, idle on its first screen | 0 ms of CPU over 10 s, working set 142 MB | |

Container stage, all passed except the package step
(`container/summary.txt`):

| Step | Result | Time |
| --- | --- | --- |
| `cargo fmt --all --check` | pass | 1 s |
| clippy, all targets, with and without the fixtures | pass | 1 s |
| tests, Rust 1.95.0, with the fixtures | 76 test runs, 1167 passed, 0 failed, 2 ignored | 521 s |
| `check --all-targets`, Rust 1.85.1 | pass | 1 s |
| tests, Rust 1.85.1, with the fixtures | 76 test runs, 1167 passed, 0 failed, 2 ignored | 562 s |
| tests, stable 1.99.0, with the fixtures | 76 test runs, 1167 passed, 0 failed, 2 ignored | 476 s |
| `cargo tree -d` | the second versions of the window stack, and `rand_core` 0.9.5 and 0.10.1 | 1 s |
| `tests/network/fail-closed.sh` (T-NET-1) | pass | 0 s |
| `tests/tor-network/two-node.sh`, `RESTART_CYCLES=3` | pass, steps 1 to 21, 63 `ok` lines | 790 s |
| workflows A to G by script | pass, 17 pictures | 39 s |
| frame measurements | at most 0.75 ms a frame after the first | 15 s |
| pictures of every fixture | 408 | 384 s |
| Linux package | exit 128: the export has no `.git`; built in the WSL clone instead | 0 s |

Clippy and the check on 1.85.1 took about a second because they reused
the build of an earlier run of the same export. Linux runs seven
Unix-only tests and Windows five Windows-only ones, so Linux has two
more. The two ignored tests are the one the private network test runs
and the KDF measurement.

WSL stage, all passed (`wsl/summary.txt`, `wsl/fuzz-summary.txt`):
`cargo deny check` (advisories, bans, licenses, sources); `cargo audit`,
445 crates, no advisory; the 60 commits of `phase-5-complete..68ac655`
all by `firebirks` as author and committer, with no trailer; the fuzz
check and build; the 20 fuzz targets each run for 26 s with no crash,
and `ui_present` (1,218,306 runs) and `ui_model_sequence` (2,407,157
runs) for 600 s each with no crash; the client photographing its own
window on WSLg in each theme. The Linux package
(`wsl/package/package.log`): `monolith-desktop-0.0.0-linux-x86_64.tar.gz`,
6,556,104 bytes, SHA-256
`d7f31d7a64812d7525e265db33fdd2035cc20106489af08c4a51cf3048d02a1d`.

Mutation (`mutation/`):

| List | Faults | Caught | Survived | Unmodified run, timeout |
| --- | --- | --- | --- | --- |
| Phase 6 | 160 | 160 | none | 651 s, 2604 s |
| Phase 5 | 57 | 57 | none | 526 s, 2104 s |
| Phase 4 | 123 | 123 | none | 439 s, 1756 s; resumed 512 s, 2048 s |
| Phase 3 | 71 | 68 | Q14, Q18, Q29 | 441 s, 1764 s |
| Phase 2 | 99 | 95 | S15, S24, CAP2, CAP3 | 452 s, 1808 s; resumed 504 s, 2016 s |
| Phase 1 | 45 | 43 | B3, B4 | 512 s, 2048 s |

555 faults, 546 caught, the nine expected survivors, none invalid; every
fault had the outcome its entry in `mutation/faults.py` expects. Each of
the 555 ids has exactly one outcome line in the logs, the first Phase 5
log kept apart (`mutation/outcomes.txt`).

## Interruptions

The mutation run was stopped four times on the same commit and went on
each time with the faults that had not ended, with the copies made again
and the unmodified run measured again; a fault in flight was run again
from the start:

- 2026-10-15 11:05: the WSL VM was restarted from outside the
  verification, after 16 faults of the Phase 5 list, all caught. The list
  was run again whole from 12:29, after the container stage
  (`final6-mutation-phase5-interrupted.log` is the first run).
- 2026-10-15 17:21, at the owner's request, after 6 faults of the Phase 4
  list; resumed on 2026-10-16 at 19:37.
- 2026-10-16 19:39, paused again while the owner decided on a review;
  resumed at 19:41.
- 2026-10-17 13:02: the process that held the run ended, after 58 faults
  of the Phase 2 list; resumed at 13:04.

A results file is written when a run of the runner ends, so a part that
was stopped left only its log lines. `results-phase4-68ac655.json` and
`results-phase2-68ac655.json` hold the part after the interruption (117
and 41 faults); the other results files the whole list. `pause/` holds
the record of the last interruption only; `final6-mutation.txt` has a
line for each.

Before any fault ran, the step of `final6-chain.log` that starts the
mutation lists was stopped at 07:26 while it waited for the container
stage, and started again at 07:27 from its own launcher
(`launch-wsl.log`); its lists began at 07:58.

## Faults caught at the timeout

- MR25 and MR26 turn a loop into one that never waits; with time paused a
  test waits for good (`docs/DESIGN_QUESTIONS.md` 12.7). As on `91bf22c`.
- MR9: `a_failed_attempt_is_not_repeated_at_once` failed first.
- CS29: twelve tests of `tests/front_end.rs`, which Phase 6 added and
  which runs before `tests/runtime.rs`, failed first. On `91bf22c` CS29
  was caught by the timeout alone.
- CS30: two tests of `tests/supervisor.rs` and one of
  `tests/front_end.rs` failed first.
- Q1: four tests of `crates/monolith-tor/tests/system.rs` failed first.
- T6 fell in the part of the Phase 2 list that the results file does not
  hold. Run again alone with one worker (`alone/`; unmodified run 551 s,
  timeout 1200 s, the least the runner allows), it was caught again, and
  340 named tests failed before the timeout.

## Container stage

The run that counts is the third, from 11:41 to 12:27 (`launch/final.log`):

- The first container, started at 07:28, was lost when the Docker Desktop
  engine stopped at 07:52. Docker Desktop could not start again until the
  WSL VM was restarted (11:05).
- The second, started at 11:06, lacked the capabilities the fail-closed
  test needs; it was stopped at 11:41 and started again with
  `--cap-add SYS_PTRACE --security-opt seccomp=unconfined`.
