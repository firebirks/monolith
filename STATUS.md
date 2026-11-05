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
| FT-E | `monolith-runtime` facade; CLI dev node commands for the private network test | done: the rules of a file of this computer moved to `monolith-storage` (`4bab5c0`), a file offered by its path and a received file saved as a new file (`export.rs`, ADR 0014), the commands of transfers in `Messenger` and in `dev-runtime` (`94fee3a`, `4d6025c`); barrier and hostile-window tests (`051af6b`) |
| FT-F | Desktop: attach by drop or typed path with the Phase 7 checks, staging and saving off the window thread, ui-model, kit progress and transfer components, screens, fixtures | done: bytes not known after a restart are `None`, never 0 (`1d0923b`); transfer cards from the runtime's state, a file dropped or typed through one dialog and one command, saving where the user chooses, progress and card components, fixtures at three sizes (`7619457`); file workflows run through the window code (`examples/files.rs`, `225eed2`, `c2e2d02`); the fuzz target of the UI model (`91af8c0`) |
| FT-G | Model and property tests, deterministic race tests, fuzz targets with seeds | done: transfers shown only as the transfer store holds them, never a change a crash could still undo (`c3af1ba`); a stopped transfer removable before its receiver is told (`8763156`); the property truth of the UI model holds transfers, and four defects it found fixed (`082e1d6`, `003c5ac`); transfers between two runtimes against a model of their own, and a crash of the receiver at every storage step (`25fd1b2`); races forced with held writes and gated sources (`b9400a7`); a hostile sender (`9314694`); fairness across contacts (`423c3e9`); fuzz target `transfer_store` with seeds (`edeb7ee`); faults FT45 to FT50 of what it found (`fb743f8`) |
| FT-H | Mutation list `PHASE8` (FT1 and on), runner and selector | done: the brief's candidates mapped to faults (`mutation/README.md`); FT51 to FT61 for the surfaces still uncovered (`5019364`), among them the defect FT-H found and fixed, messages written before they were durable (`fea2af2`); tests of a chat message after at most one chunk and of a nonce for every segment (`365ceac`); fuzz target `transfer_sequence` with seeds (`de9f1cf`, `d4e9479`); a receiver bounded through lost sessions, repeated offers and a full disk (`d8c5886`); the whole list, FT1 to FT61, caught on `71d5c5a` |
| FT-I | Private Tor network steps for files; Windows path and save scenarios; measurements | implemented; the whole run is repeated on the frozen commit in FT-L. Steps 22 to 32 of `two-node.sh` (`93706ed`, `5dc5297`, `812d7fd`): both peers sending and receiving, crashes of each, resume, chat both ways, the saved bytes compared by SHA-256; a peer built from the tag `phase-7-complete` (step 31); a key rotation and a contact made again (step 32, `tests/tor_network.rs`); a fault proxy that cuts connections mid-frame and holds data back (step 27); the network isolation checks of every node run (step 24). Steps 1 to 23 passed on `812d7fd` and on `98cd6ac`; on `98cd6ac` step 25 failed on a race of the script, not of the code (`c7b6a37`). That the bounded test's `Failed(Digest)` cannot happen in production: FT62 and its test. Windows: a Unicode name deep in folders (`b2ce2e3`), GUI scenario L |
| FT-J | Documents: README, ARCHITECTURE, PROTOCOL, SECURITY_INVARIANTS, RESOURCE_LIMITS, STORAGE, CRYPTOGRAPHY, TEST_PLAN, THREAT_MODEL, DESIGN_QUESTIONS 15, DESKTOP, fuzz, mutation and Tor network READMEs | done (`d192bde` to `7e10e07`, and with the fixes of FT-K) |
| FT-K | Independent reviews: protocol and security, persistence, runtime and concurrency, filesystem and platform, UI trust | done: no high finding; every finding reproduced before its fix, with a regression test and a fault (FT63 to FT80); one refuted by a crash sweep, two not reproduced and made safe, three kept and documented (`DESIGN_QUESTIONS.md` section 15.3) |
| FT-L | Freeze, final verification, report | open |
