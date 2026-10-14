# Final verification of 91bf22c

This directory holds the outputs of the final verification of Phase 5 on
the frozen commit `91bf22c` (`91bf22cd60930e52f53229bde389e8f115927e64`)
of `phase-5-messaging-runtime`, and the scripts that ran it. They are
kept on the branch `verification-91bf22c`, which changes nothing outside
this directory and is not merged into `main`. The final report is section
6 of the status file of Phase 5 (`STATUS.md` at `78881c1`); the owner
accepted it on 2026-10-14. `docs/DESIGN_QUESTIONS.md` 12.8 records the
completion.

Local paths in the logs are written as `<repo>` (the checkout or copy of
the commit), `<home>` (the home directory in the Linux VM), `<winhome>`
(the home directory on Windows) and `<work>` (the directory of the
scripts). The box drawing of `cargo tree` is written in ASCII.

## Environment

- One machine, Windows 11 with WSL2: a Debian 13.4 VM with 4 CPUs and
  12 GB, kernel 6.18.33.2-microsoft-standard-WSL2, and Docker Desktop for
  the container.
- Container (`scripts/Dockerfile`): `rust:1.95-bookworm` (Debian 12),
  Rust 1.95.0, 1.85.1 and stable 1.99.0, Tor 0.4.9.13 from the Tor
  Project's repository, Chutney at `ae3a33c`, strace 6.1. The commit was
  exported without `.git` and each of its 362 files compared with the
  commit.
- WSL: a clone at the commit, Rust 1.95.0, nightly 1.101.0 (2026-10-10)
  with cargo-fuzz 0.13.2, cargo-deny 0.20.2, cargo-audit 0.22.2. The
  mutation runner ran in a second clone at the commit, with two workers.

## Dates

- Container stage: 2026-10-12, 16:41 to 17:47.
- WSL stage: 2026-10-12, 17:47 to 18:37.
- Mutation lists: 2026-10-12 18:37 to 2026-10-14 16:11, with three
  interruptions (below).
- Faults run again alone, and the check of CS29: 2026-10-14, 16:13 to
  17:37.

## Layout

- `container/`: `summary.txt` (a line per step), `environment.txt`, a log
  per step and the log of the container.
- `wsl/`: `summary.txt`, `environment.txt`, `deny.log`, `audit.log`,
  `identity.log`, the fuzz build and every fuzz run
  (`fuzz-summary.txt`).
- `mutation/`: `final-mutation.txt` (a line per list or part of a list),
  the log of every part (`final-mutation-phaseN.log`, and
  `...-resumed.log` for the part after an interruption), the results
  files, and what each interruption recorded (`pause-1-phase5`,
  `pause-2-phase3`, `restart-phase2`: when, which list, the faults left).
- `alone/`: MR9, Q1 and T6 run again alone, and the supervisor tests run
  on their own under CS29.
- `first-75ccf7c/`: the first final verification, of `75ccf7c`, which
  stopped when MR28 survived.
- `scripts/`: the scripts that ran all of this.

## Results

Container stage, all passed (`container/summary.txt`):

| Step | Result | Time |
| --- | --- | --- |
| `cargo fmt --all --check` | pass | 1 s |
| clippy, all targets, warnings denied | pass | 24 s |
| tests, Rust 1.95.0 | 39 binaries, 782 passed, 0 failed, 2 ignored | 611 s |
| `check --all-targets`, Rust 1.85.1 | pass | 17 s |
| tests, Rust 1.85.1 | 39 binaries, 782 passed, 0 failed, 2 ignored | 628 s |
| tests, stable 1.99.0 | 39 binaries, 782 passed, 0 failed, 2 ignored | 604 s |
| `cargo tree -d` | only `rand_core` 0.9.5 and 0.10.1 | 0 s |
| `tests/network/fail-closed.sh` (T-NET-1) | pass | 25 s |
| `tests/tor-network/two-node.sh`, `RESTART_CYCLES=15` | pass, all 20 steps, 60 `ok` lines | 2085 s |

WSL stage, all passed (`wsl/summary.txt`, `wsl/fuzz-summary.txt`):
`cargo deny check` (advisories, bans, licenses, sources); `cargo audit`,
83 crates, no advisory; the 21 commits of `phase-4-complete..91bf22c`
all by `firebirks` as author and committer; the 18 fuzz targets built,
each run for 25 s with no crash, and `message_store`,
`message_sequence`, `vault_payload` and `contact_store` for 600 s each
with no crash and no artifact.

Mutation (`mutation/`):

| List | Faults | Caught | Survived |
| --- | --- | --- | --- |
| Phase 5 | 57 | 57 | none |
| Phase 4 | 123 | 123 | none |
| Phase 3 | 71 | 68 | Q14, Q18, Q29 |
| Phase 2 | 99 | 95 | S15, S24, CAP2, CAP3 |
| Phase 1 | 45 | 43 | B3, B4 |

395 faults, 386 caught, the nine expected survivors, none invalid; every
fault had the outcome its entry in `mutation/faults.py` expects. Each of
the 395 ids has exactly one outcome line in the logs.

## Interruptions

The mutation run was stopped three times on the same commit and resumed
each time from the faults that had not ended, with the copies made again
and the unmodified run measured again; a fault in flight was run again
from the start:

- 2026-10-12 21:31, at the owner's request, in Phase 5 after 24 faults;
  resumed at 22:04.
- 2026-10-13 18:48, at the owner's request, in Phase 3 after 51 faults;
  resumed at 22:03.
- 2026-10-14 about 04:20, a restart of the machine for updates, in Phase
  2 after 37 faults; resumed at 08:11.

A results file is written when a run of the runner ends, so a part that
was stopped left only its log lines. `results-phase5-91bf22c.json`,
`results-phase3-91bf22c.json` and `results-phase2-91bf22c.json` hold the
part after the interruption; `results-phase4-91bf22c.json` and
`results-phase1-91bf22c.json` the whole list.

## Faults caught at the timeout

- CS30, MR25 and MR26 turn a loop into one that never waits; with time
  paused a test waits for good (`docs/DESIGN_QUESTIONS.md` 12.7). CS30
  also made its two supervisor tests fail.
- MR9, Q1 and T6 fell in stopped parts, so their failing tests were not
  recorded. Run again alone with one worker (`alone/`), MR9 made
  `a_failed_attempt_is_not_repeated_at_once` fail, Q1
  `without_a_socks_endpoint_there_is_no_connection_at_all`, and T6 324
  named tests; each also reached the timeout of 1200 s.
- CS29: under it the wait for publication of `Peer::start` in
  `tests/runtime.rs`, which runs before `tests/supervisor.rs`, lasts
  hours, so the supervisor tests were not reached. Run on their own in a
  mirror of the commit with CS29 applied
  (`alone/final-cs29-named.txt`), the four tests that caught it on
  `970316f` fail.
