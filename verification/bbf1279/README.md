# Final verification of Phase 7: 740b671, then bbf1279

This directory holds the outputs of the final verification of Phase 7
and the scripts that ran it. They are kept on the branch
`verification-bbf1279`, which changes nothing outside this directory and
is not merged into `main`.

- The full final verification ran on the frozen commit `740b671`
  (`740b6715c5e1c3c488f3397624d6eb764e763eec`) of
  `phase-7-security-hardening`.
- The verified commit is `bbf1279`
  (`bbf1279439ea9f58703e10415f2ffa2f5321eeed`). Its production code,
  manifests and lockfile are those of `740b671`; it changes two
  regression tests, the mutation selector, the dependency policy and
  documents. What those changes can alter was verified again, first on
  `ba12472`, then on `bbf1279`.

The final report is section 5 of the status file of Phase 7
(`STATUS.md`, kept in the history of the branch); the owner accepted it
on 2026-10-24. `docs/DESIGN_QUESTIONS.md` 14.3 records the completion.

The two packages are not here; their sizes and SHA-256 are below and the
`.sha256` files are kept.

Local paths in the logs are written as `<repo>` (the checkout, clone or
copy of the commit), `<home>` (the home directory in the Linux VM),
`<winhome>` (the home directory on Windows), `<work>` (the directory of
the scripts), `<host>` (the name of the machine) and `<user>` (the local
user in listings). Box drawing is written in ASCII.

## Environment

- One machine, Windows 11 with WSL2: a Debian VM, kernel
  6.18.33.2-microsoft-standard-WSL2, with 8 CPUs and 12 GB for the
  verification, and Docker Desktop for the container.
- Windows: a detached worktree at the commit with a target directory of
  its own; Rust 1.95.0 and 1.85.1.
- Container (`scripts/full-740b671/container/Dockerfile`, image
  `monolith-verify:phase7`): Rust 1.95.0, 1.85.1, 1.99.0 and 1.99.1,
  Tor 0.4.9.13, Chutney at `ae3a33c`, strace 6.1. The commit was exported
  without `.git` (`git archive`) and marked with its hash.
- WSL: a clone at the commit, Rust 1.95.0, nightly 1.101.0 (2026-10-10)
  with cargo-fuzz 0.13.2, cargo-deny 0.20.2, cargo-audit 0.22.2. The
  Linux package and the mutation lists ran in it, four workers.

## Dates

- 740b671: Windows stage 2026-10-19 17:41 to 18:16; container and WSL
  stages the same evening; Windows mutation list until 19:11; Linux
  mutation lists 2026-10-19 20:15 to 2026-10-21 02:59, interrupted once
  (WSL stopped at 05:22 on 2026-10-20, during the Phase 4 list; the 86
  faults not ended ran again from the start).
- ba12472: 2026-10-21, checks from 08:39, mutation 11:51 to 22:45.
- Diagnosis of the store crash tests: 2026-10-22.
- bbf1279: checks 2026-10-22 12:58 to 13:18, test B to 13:30, the
  Phase 7 list to 14:24; paused by the owner; the other lists 2026-10-23
  22:13 to 2026-10-24 06:10.

## Layout

- `full-740b671/`
  - `windows/`: `summary.txt`, a log per step, the pictures of every
    fixture (`screens/`) and of the workflows (`scenarios/`), the release
    client photographing its own window, the idle client, the checksum of
    the Windows package.
  - `container/`: `summary.txt`, `environment.txt`, a log per step
    (`two-node.log`, `fail-closed.log`, `tree-d.log` among them), the
    pictures.
  - `wsl/`: `summary.txt`, `environment.txt`, `deny.log` (failed on three
    advisories), `deny-again/` (deny and audit on `d22adda`, passed),
    `audit.log`, `identity.log`, `trailers.log`, the fuzz runs
    (`fuzz-summary.txt`), the client on WSLg, `package/`.
  - `mutation/`: `final7-mutation.txt`, a log per list
    (`...-phase4-interrupted.log` and `...-phase4-resumed.log`), the
    results files, `outcomes.txt`, and the Windows list
    (`mutation-phase7w.log`, `results-phase7w-740b671.json`).
  - `launch/`: the logs of the chains that ran the stages.
- `targeted-ba12472/`: the checks on Windows and in the container, the
  selection (`mutation/affected.txt`), the mutation run, deny and audit
  (`deny/`).
- `diagnosis/`: the runs that told the harness bound from a defect: the
  store tests alone (`A*`), four copies at once (`B*`), four copies with
  the bound at 3600 s (`D/`), one core with separate binaries (`C3b/`),
  `summary.txt`, `panics.txt`, and the memory of Windows during `D`.
- `targeted-bbf1279/`: the checks on Windows and in the container, test B
  (`test-b/`), the selection and the mutation run (`mutation/`,
  `paused-1.txt` empty: no fault ran during a pause), and the log of the
  watchdog of memory and disk (`launch/`).
- `scripts/`: the scripts that ran all of this.

## Results on 740b671

| Stage | Windows | Linux |
| --- | --- | --- |
| fmt, clippy, clippy with fixtures, `cargo doc` | pass | pass |
| tests with the fixtures, 1.95.0 | 76 test runs, 1264 passed, 0 failed, 2 ignored | 76 test runs, 1252 passed, 0 failed, 2 ignored |
| `check --all-targets`, 1.85.1 | pass | pass |
| tests, 1.85.1 | 1264 passed, 0 failed, 2 ignored | 1252 passed, 0 failed, 2 ignored |
| tests, 1.99.1 | - | 1252 passed, 0 failed, 2 ignored |
| `cargo tree -d` | - | pass |
| T-NET-1, `tests/network/fail-closed.sh` | - | pass |
| `tests/tor-network/two-node.sh` | - | pass, steps 1 to 21, 1073 s |
| workflows A to G by script | pass | pass |
| pictures of every fixture, frame measurements | pass | pass |
| `tor_smoke` on the in-memory Tor | pass | - |
| packaged client, idle | 16 ms of CPU over 10 s, working set 150 MB | - |
| package | `monolith-desktop-0.0.0-windows-x86_64.zip`, 4,575,549 bytes, SHA-256 `534b6ae2dc5f57204d3114842a9ee63ee722c72179bfd738fc2130bb71015658` | `monolith-desktop-0.0.0-linux-x86_64.tar.gz`, 6,580,134 bytes, SHA-256 `964672c3e57ddd06663a65b7b208a472559a52368ae91793ee681bfe46d0da1e` |

WSL: deny failed on RUSTSEC-2026-0370, 0371 and 0372 (unmaintained D-Bus
crates under the accessibility bridge); with the owner's approval they
are ignored with their reasons (`fce59a1`, `d22adda`), and deny and audit
passed on `d22adda`, production code and lockfile unchanged. Fuzz: 20
targets for 26 s each and five for 600 s each, no crash.

Mutation: 626 faults for Linux, each with exactly one outcome; caught
except B3, B4, S15, S24, CAP2, CAP3, Q14, Q18, Q29 (expected) and MR36;
caught only by the timeout: MR9, MR25, MR26, CS30, Q1. Windows list: 14
of 14 caught.

## Results on ba12472 and bbf1279

On both, on Windows (1.95.0, 1.85.1) and in the container (1.95.0,
1.85.1, 1.99.1): fmt and clippy pass, and the tests of `monolith-core`
give 325 passed, 0 failed, 1 ignored on each. On `ba12472` deny and
audit passed, and MR36 was caught by its test in 545 s.

On `ba12472` the crash tests of `monolith-core/tests/store.rs` reached
the 120 s bound of their harness under four workers, so B3, B4, Q14, Q18
and Q29 looked caught; `diagnosis/` shows the cause, and `801bcbf` gave
those tests a bound of their own. On `bbf1279`: test B, four copies of
the store tests at once, passed 12 of 12; the 78 faults the selector
chose each have exactly one outcome, the same as on `740b671`; the nine
expected survivors survived with no test failing, and no fault was caught
by the tests of `tests/store.rs` alone.
