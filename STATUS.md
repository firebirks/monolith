# Status of the Phase 8 work

Written on 2026-10-24, when Phase 8 started, so that the work can be
picked up on another machine. This file describes work in progress on the
branch `phase-8-file-transfer`. It is removed when Phase 8 is closed; it
does not belong on `main`.

Branch: `phase-8-file-transfer`, created from `main` at `b93a10a`
(`phase-7-complete`). The production code of Phase 7 was verified on
`bbf1279`, whose production code is that of `740b671`.

## 1. What Phase 8 is

Secure, resumable, bounded file transfer between accepted contacts: a
file is chosen, staged encrypted, offered, explicitly accepted or
rejected, sent in chunks over the authenticated session over Tor,
resumed across lost sessions and restarts, verified and staged before
the completion is acknowledged, shown with its progress in the desktop
client and saved where the user chooses. A feature phase: no previews,
no media, no cloud, no groups, no folders, no automatic opening.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history is
  rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 7 are a frozen baseline: no old test is weakened, no fault
  of an earlier list is renumbered or weakened; a pattern that moves
  keeps its defect exactly.
- Each defect: reproduce it, a regression test that fails before the
  fix, the smallest safe fix, a look at sibling patterns, a mutation
  fault where useful. Deterministic schedules and hooks, not sleeps.
- The Phase 8 mutation list is `FT1` and on.
- Independent reviews are made each blind to the others; documents do
  not name how they were made.
- Different source variants build in separate target directories; a
  fault counts as caught only when a test that bears on it fails
  (`mutation/README.md`).
- The phase is complete only after its final verification on one frozen
  commit, a full mutation baseline of Phases 1 to 8, and the final
  report.

## 2. Design

- `docs/adr/0013-file-transfer.md`: the amended exchange of version 1
  (the label stays; FileAccept gains an offset; transfers outlive
  sessions; offers repeated on every session; completion order; resume;
  cancellation; lifecycle; scheduling; limits).
- `docs/adr/0014-transfer-staging.md`: the transfer store and the
  encrypted blobs (keys from the vault key, random-nonce segments, write
  order, cleanup, ephemeral installations, saving).

Both were reviewed independently before any code relied on them: the
protocol design in three rounds, the staging format in three; every
finding was taken in.

## 3. Work items

| ID | Item | Status |
| --- | --- | --- |
| FT-A | ADR 0013 and 0014, reviewed | done |
| FT-B | `monolith-protocol`: FileAccept with an offset, limits, `PROTOCOL.md` 8, 8.9, 13 | done (`0a6aaf2`; also declares the `alloc` feature of `zeroize` the crate uses, which it built without only inside the workspace) |
| FT-C | `monolith-storage`: streaming file access (Unix, Windows, memory with crash model), blob format, transfer store, keys, test vectors | done (`7dd86f0` and the next commit); documents in FT-J |
| FT-D | `monolith-core`: transfer state machine, durable records, session pump and answers, scheduler and linger, lifecycle, commands, events, queries | done (`21f73b5`, `31e5ed5`, `3f53ed3`, `bafc137`); tests in `tests/transfers.rs` and `tests/transfer_restarts.rs`; FT1 to FT33 and the moved faults of earlier lists caught (`31e5ed5`, `edf739f`) |
| FT-E | `monolith-runtime` facade; CLI dev node commands for the private network test | open |
| FT-F | Desktop: attach by drop or typed path with the Phase 7 checks, staging and saving off the window thread, ui-model, kit progress and transfer components, screens, fixtures | open |
| FT-G | Model and property tests, deterministic race tests, fuzz targets with seeds | open |
| FT-H | Mutation list `PHASE8` (FT1 and on), runner and selector | in progress: FT1 to FT33 (protocol, storage, core) with the runner and the selector (`31e5ed5`, `edf739f`); faults of the later items to come |
| FT-I | Private Tor network steps for files; Windows path and save scenarios; measurements | open |
| FT-J | Documents: README, ARCHITECTURE, PROTOCOL, SECURITY_INVARIANTS, RESOURCE_LIMITS, STORAGE, CRYPTOGRAPHY, TEST_PLAN, THREAT_MODEL, DESIGN_QUESTIONS 15, DESKTOP, fuzz, mutation and Tor network READMEs | open |
| FT-K | Independent reviews: protocol and security, persistence, runtime and concurrency, filesystem and platform, UI trust | open |
| FT-L | Freeze, final verification, report | open |
