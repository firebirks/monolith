# Final verification of 970316f

This branch holds the outputs of the final verification (STATUS.md
section 5.2) of the frozen candidate `970316f` on `phase-4-contact-store`,
and the script that runs the mutation phases still to run. It is a work
branch: it is not merged and the candidate is not changed.

Local paths in the logs are written as `<repo>`, `<home>` and `<out>`.

## Stage 1: checks

All passed; see `stage1/summary.txt` and the logs beside it.

- fmt, clippy with warnings denied, cargo deny (advisories, bans,
  licenses, sources ok).
- Tests on 1.95.0, 1.85.1 and stable: 33 binaries, 682 passed, 0 failed,
  2 ignored each. `cargo check` on 1.85.1 passes.
- cargo audit: 82 dependencies, no vulnerabilities.
- `cargo tree -d`: only `rand_core` 0.9.5 (proptest, dev) and 0.10.1
  (x25519-dalek), as recorded in deny.toml.
- Fuzz targets build; T-NET-1 (fail-closed network) passed.
- 171 commits since `2f27dcb`, all with firebirks as author and committer.

## Stage 2: fuzzing and the private Tor network

All passed; see `stage2/fuzz-summary.txt` and `stage2/tor.log`.

- Smoke run of all 16 fuzz targets: exit 0, no crashes.
- 600 s runs of credential_sequence, contact_store, vault_payload,
  session_frames, handshake_initiator, handshake_responder,
  tor_control_reply and socks_reply: exit 0, no crashes, no new
  artifacts.
- `tests/tor-network/two-node.sh` with `RESTART_CYCLES=15`: all 12 steps
  passed; 15 of 15 restart cycles recovered in 1 to 20 s with one attempt
  and SOCKS reply 0x00.

## Stage 3: mutation

See `stage3/summary.txt`, the logs and the results files.

| Phase | Faults | Caught | Survived | Runner exit |
|-------|--------|--------|----------|-------------|
| 1 | 45 | 43 | B3, B4 | 0 |
| 2 | 99 | 95 | CAP2, CAP3, S15, S24 | 0 |
| 3 | 71 | to run | expected Q14, Q18, Q29 | |
| 4 | 123 | to run | expected none | |

Every survivor so far is one the manifest lists with its reason. No
fault was invalid.

In phase 2, T6 was caught at the 1200 s timeout while four workers shared
the CPUs. Run alone it was caught in 791 s by failing tests
(`stage3/results-phase2-970316f-T6.json`), so the catch does not depend
on the timeout.

## Running phases 3 and 4

From a checkout of this branch, with the toolchains of the repository
installed:

    verification/970316f/run-phases.sh 4

The script makes a detached worktree of `970316f` next to the
repository (`../monolith-970316f`, or the path in `TREE`), checks that
it is clean, and runs phase 3 then phase 4 in it with the given number of
workers. Logs, results and a line per phase go to `stage3/`. `CPUS=0-10`
keeps the runs on those CPUs. With 4 workers on 12 CPUs, phase 3 should
take about 2.5 hours and phase 4 4 to 5 hours; the first build of each
worker copy adds some minutes.

For each phase, check that the runner exit is 0, that the survivors are
the expected ones, and list any fault whose `failed` holds `(timeout)`;
rerun such a fault alone, for example:

    (cd ../monolith-970316f && python3 mutation/run.py phase4 1 CS17)

A run with fault ids writes the same results file name as the whole
phase, so copy the phase file aside first.

## Draft of the final report

`phase4-final-report-draft.md` is a draft of the Phase 4 final report
(the 26 items of the brief). Results not known yet are marked
`[PENDING: ...]` and points for the owner `[OWNER DECISION: ...]`. It
becomes `STATUS.md` section 5.3 only after every stage of the final
verification has completed; until then Phase 4 is not complete.
