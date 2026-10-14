# Status of the Phase 6 work

Written on 2026-10-14, while Phase 6 is in progress, so that the work can
be picked up on another machine. This file describes work in progress on
the branch `phase-6-desktop-client`. It is removed when Phase 6 is
closed; it does not belong on `main`.

Branch: `phase-6-desktop-client`, created from `phase-5-complete`
(`3be1254`), whose production code is that of `91bf22c`, the commit on
which Phase 5 was verified. The mutation faults of Phases 1 to 5 (MR1 to
MR57 and CS1 to CS123 among them) and their regression tests were all
present at the start.

## 1. What Phase 6 is

The desktop client: egui and eframe over the runtime of Phase 5, with a
design system of its own, so that a person can use Monolith without the
CLI, on Linux and on Windows. `docs/DESKTOP.md` describes it;
`docs/DESIGN_QUESTIONS.md` section 13 has the brief in short and the
decisions (P6-1 to P6-19); the ADRs are `docs/adr/0010-gui-toolkit-egui.md`,
`0011-ui-security-boundary.md` and `0012-design-system.md`.

Rules that hold until the phase is closed:

- Work on this branch only. `main` is not moved, nothing is pushed,
  merged or tagged unless the owner asks for it. No history is rewritten.
- Every commit is made as `firebirks
  <336470935+firebirks@users.noreply.github.com>`.
- Phases 1 to 5 are a frozen baseline: no old test is weakened, no fault
  of an earlier list is renumbered or weakened. CS40 and CS41 point at
  `disk_unix.rs` since the Unix code of the vault directory moved there;
  their text is unchanged.
- The phase is complete only after its final verification on one frozen
  commit and the final report.

## 2. Building

The whole workspace builds and its tests pass on Linux and on Windows
(P6-13). The client: `cargo run --release -p monolith-desktop`; the
pictures of every screen: `cargo run -p monolith-ui --features fixtures
--example screens -- <dir>`; the workflows A to G by script:
`cargo run --release -p monolith-desktop --example scenarios -- <dir>`;
the smoke test of step 21 of the private Tor network test, on the
in-memory Tor: `cargo run --release -p monolith-desktop --example
tor_smoke`; the frame costs: `cargo run --release -p monolith-ui
--features fixtures --example measure`; the archives:
`packaging/README.md`. The mutation
runner and the private Tor network test run on Linux only.

## 3. What is done

- Runtime additions of P6-4 (requests, invitations, names, verification,
  request mode, service state, Tor check) with `tests/front_end.rs`.
- Windows: the vault directory (`disk_windows.rs`, `STORAGE.md` 3.8).
- The UI model, the design system (components, fonts, icons, theme), the
  screens, the bridge and the window; Tails places (P6-16).
- Tests: the model scenarios of the brief's section 42, the components,
  contrast, the screens (source scan, smallest window, idle, keyboard),
  the bridge against the runtime, the boundary, the files; two fuzz
  targets (three faults of `present.rs` found and fixed); the Phase 6
  mutation list UI1 to UI34; the workflows A to G by script; step 21 of
  the private Tor network test (two clients driven through their window
  code over Tor A and Tor B, `examples/tor_smoke.rs`; not yet run against
  Tor); packaging for Linux and Windows; the documents.

## 4. What was verified

Not the final verification yet. On the way:

- Phase 6 mutation list: on `7e48ce2`, UI1 to UI12 and UI14 to UI26
  caught, UI13 survived; on `21b3df0`, UI13 (with the test of `1b2971a`)
  and UI27 to UI32 and UI34 caught, UI33 survived. UI33 removed the scrim's
  interaction and the modal layer together, but the scrim's own widget
  still shields the pointer; it is now the modal layer alone (the keyboard
  reaching behind a modal), with a test that catches it (`557863d`). Not
  yet run in its new form.
- Workflows A to G passed against the runtime on the in-memory Tor.
- Every test of the workspace passes on Windows (941, on `b2b20e1`) and,
  earlier, on Linux.
- The release binary on Windows: 0 ms of CPU over 10 s idle, 117 MB.

## 5. Reviews

Four independent reviews were started on `21b3df0`: A visual, B UX, C
security and architecture, D egui. A, B and C have reported; D had not
when this was written.

C's findings (nothing critical or high; the handles and the stale-answer
design hold, and the runtime additions keep the Phase 5 guarantees):
medium: on Windows the default Tor is Tor Browser, usually closed, so
another local account can bind 127.0.0.1:9150 and receive the onion names
of contacts at every delivery attempt; dial only while an authenticated
control session exists and check Tor's SOCKS listener, and document the
risk. Low: the composer's undo history keeps sent text, also after the
messages are deleted; a session that saves nothing still writes the
settings file; a card file read is applied to whichever Add contact
dialog is open; the window never learns that the bridge thread failed,
and a failed task leaves its command unanswered; passphrase hygiene (no
character limit, egui's per-frame copies, input methods, an over-long
unlock passphrase that shows the create screen); `Debug` of the model
prints cards and drafts; the Windows vault directory is described wrongly
(std renames with `MoveFileExW` first, `FileRenameInfoEx` only after
access is denied; `4918c9a` stated the reverse) and the durability of a
rename through a directory flush is not documented by Windows.
Informational: a card path is checked then opened by name (a FIFO swapped
in can hang the exit); UNC paths make Windows connect over SMB; copied
invitation cards enter the Windows clipboard history.

A's findings, to reproduce, test and fix: Settings breaks below 880
points (nav always shown, rows clipped, the header button over its
caption); dialogs have no maximum height and do not scroll (at 760 by 520
the footer of contact details is off screen); a failed store is said three
times and its toast covers the composer's line; toasts sit bottom centre
over the composer and modal footers; the empty identity repeats its empty
state in the sidebar; the narrow list shows neither Tor nor a failed store;
in the dark theme borders vanish on raised surfaces, secondary and disabled
buttons and the segmented control nearly disappear; dark danger buttons
(3.1 to 1) and the light warning pill (4.1 to 1) fail contrast and are not
in the test; field errors are drawn away from their fields
(`TextField::error` is unused); an identity's avatar colour differs between
the sidebar and the settings (full against compact fingerprint); reading a
card from a file means typing a path; minor: the composer's permanent
focus ring, a small and dense QR code with a stray pixel beside it,
labels selectable by default, initials of placeholder names, technical
first screens, fingerprint wrapping, inconsistent buttons, content cut at
scroll edges, hidden scroll bars, an empty sidebar before the first
identity, punctuation. The fixtures miss the open switcher, the narrow
conversation and requests panes, and two show the wrong content.

B's findings, to reproduce, test and fix:

1. A plain card (not an invitation) shared one way fails silently under the
   default request mode "invitations only": the request is dropped, nobody
   is told, the sender waits for ever. The client never says whether an
   added text was a card or an invitation. (Blocker.)
2. Making an identity needs Tor; its failure says nothing about what to do.
   The Windows cookie path of Tor Browser misses a Desktop under OneDrive.
3. Block cannot be undone in the client, is one click on a request card and
   first in its focus order; the failure "Unblock them first" names an
   action that does not exist.
4. "An invitation always works, whatever you choose" is false for "Nobody".
5. Invitations are described as for one person; they admit requests until
   revoked.
6. The request the other side sees carries no name or message; the texts
   suggest otherwise.
7. An offline contact and Tor being down are hard to tell apart; the top
   bar's Tor pill reads as the contact's presence; a contact waiting for
   acceptance gets "waiting for them to come online".
8. Nothing says Monolith has to stay open on both sides for delivery;
   closing never warns, also when a session that saves nothing would lose
   everything.
9. Messages and requests of another identity leave no trace on screen.
10. One's own card is reachable only from the settings.
11. "Delete messages" does not say that undelivered messages will not be
    sent.
12. The text of a new identity promises unlinkability the design rules out.
13. Settings > Identities is clipped at 820 and 760 points wide.
14. Minor: Tor state not shown in the narrow list; fingerprints never say
    where the other person finds theirs; Decline and Block toasts; the
    Remove and Delete identity texts; statuses only as icons to sighted
    users; Tor settings restart; error sentences without a next step;
    requests and unread marks lost on restart; toasts over the composer;
    wording nits; the contact dialog mixes instant and saved changes.

## 6. What is left, in order

1. The report of review D; then for every finding of the four:
   reproduce it, a regression test before the fix, a mutation fault where
   useful, a review of sibling patterns.
2. The property tests of the UI model (`tests/properties.rs`), under way.
3. Pictures of the workflows taken again after the fixes.
4. The final verification on one frozen commit: fmt, clippy, every test on
   both toolchains and on Windows, deny, audit, every fuzz target, the
   mutation lists of Phases 1 to 6, the fail-closed and private Tor tests
   (in Docker), the workflows, the packages, the measurements, a visual
   review.
5. The final report (36 items), ending in `PHASE 6 COMPLETE` or
   `PHASE 6 NOT COMPLETE`.
