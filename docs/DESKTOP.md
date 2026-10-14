# The desktop client

The desktop client is how a person uses Monolith without the command line:
one window over the runtime of Phase 5. This document says how it is built,
what it shows and why, and what it does not do. The decisions behind it are
in `adr/0010-gui-toolkit-egui.md` (toolkit), `adr/0011-ui-security-boundary.md`
(boundary), `adr/0012-design-system.md` (design system) and
`DESIGN_QUESTIONS.md` section 13 (P6-1 and on).

## 1. Crates

    monolith-runtime        Messenger: commands, queries, events (Phase 5)
            |
    monolith-desktop        the binary: eframe, the bridge, the files it writes
            |
    monolith-ui             the screens and the shell of the window
       |          |
    monolith-ui-model   monolith-ui-kit
    (state, no egui)    (design system on egui)

`monolith-ui-model`, `monolith-ui-kit` and `monolith-ui` depend on no
Monolith crate; `crates/monolith-desktop/tests/boundary.rs` checks every
path of the dependency graph. `monolith-desktop` uses `Messenger` and the
plain values of `monolith_runtime::api` only, and holds no object of the
core: no local identity, link, session, key, capability or vault.

## 2. Platforms

- Linux, x86_64, X11 or Wayland, OpenGL 2.1 or OpenGL ES 2.0. The primary
  platform; Tails and Whonix as in `PLATFORM_TAILS.md` and
  `PLATFORM_WHONIX.md`.
- Windows 10 and 11, x86_64, OpenGL 2.1. The whole stack runs there:
  runtime, storage (`STORAGE.md` section 3.8) and Tor through Tor Browser
  or the Tor Expert Bundle (P6-13).
- macOS is not built or tested.

How to build and package: `packaging/README.md`. How a user points
Monolith at Tor: `packaging/linux/README.txt`, `packaging/windows/README.txt`.

## 3. The command and event boundary

The runtime runs on a tokio runtime of its own thread
(`crates/monolith-desktop/src/bridge.rs`). The window and the bridge talk
through two bounded channels of 256 entries; the window never waits on
either. A command that does not fit waits in a queue of the window until a
later frame; an update waits in the bridge until the window takes it; a
window that does not take its updates (minimized, say) makes the bridge
fall behind on events, which the runtime reports as missed, and the model
reads everything again (section 5).

Each frame the window takes the updates and applies them to the model,
draws, and hands the commands the model issued to the bridge
(`crates/monolith-desktop/src/window.rs`). Nothing in a drawing path awaits
anything: disk, Tor, an acknowledgement, the vault and the shutdown of the
runtime all happen on the bridge's thread, and the files the screens ask
for on a thread of their own (`files.rs`).

The bridge runs queries and forwards events in one task, in order, so that
the model sees them in an order the runtime went through. A command that
writes or waits (sending, accepting, revoking, checking Tor) runs as a task
of its own and reports back through that same task. Reading one identity
(contacts, conversations, requests, invitations, the state of its service,
its request mode) is one run of queries with nothing in between.

Every command carries the handles it acts on (`UiIdentityId`,
`UiContactId`, `UiMessageId`, `UiRequestId`, `UiInvitationId`), and its
answer names the command. Identity, contact and message handles hold the
durable identifiers of the runtime. Requests and invitations, whose
references in the core are local to one instance of an identity, get a
number from the bridge, never given twice in a process; the bridge checks
on every use that the number belongs to the identity the command names, so
a number of one identity names nothing of another.

Errors of the runtime are mapped to one of the fixed sentences of
`monolith_ui_model::Failure` (`crates/monolith-desktop/src/convert.rs`).
No `Debug` or `Display` of an error, and no message, name or card, reaches
the screen, a log or the settings file.

## 4. What is shown, and from where

Everything authoritative on screen (identities, contacts, conversations,
messages and their status, requests, invitations, the state of Tor) is
what a query of the runtime answered, kept per identity. The model never
changes it on its own. Events are notifications: one makes the model read
again the part of the state it names, for the identity it names; many
events about one part make one read. Transient state belongs to the window:
the selection (a handle), the open dialog (with the handles it captured when
it opened), drafts, the filter, focus.

### Queued and Delivered

A message is shown with exactly the status the runtime gives it: Queued
(a clock), Delivered (two ticks), or received (no mark). Delivered comes
only from the runtime's answer for that message, which follows an
acknowledgement made durable by the contact (`adr/0007-delivery-semantics.md`).
Pressing Send, the command returning, or a session existing is never shown
as delivery. A draft is cleared only once the runtime says the message is
durably queued; a refused send keeps the draft and says why.

### Late and stale events

These hold, each with a test in `crates/monolith-ui-model/tests/model.rs`
and a mutation fault (`mutation/faults.py`, UI1 to UI15):

- a late event or answer of one identity changes nothing shown of another;
- a contact made again (a new record for the same person) inherits nothing
  of the old record, and a late event of the old record does not touch it;
- a dialog acts on the handles it captured, not on what is selected when it
  is confirmed;
- an action on a request that was replaced names the old request, which
  the runtime refuses;
- a Delivered for a message of another conversation, or a position in a
  list, is never taken for a message shown;
- an answer to no command in flight changes nothing;
- after missed events everything is read again.

Property tests (`tests/properties.rs`) run random sequences of actions,
events and late answers against a model of the runtime written apart from
the UI model, and a fuzz target (`fuzz/fuzz_targets/ui_model_sequence.rs`)
runs arbitrary ones.

## 5. Tor, and being offline

The top bar shows one of four states (`TorIndicator`): starting, Connecting
(Tor answers but builds a circuit, or the service of the identity is being
published), Connected through Tor (the service of the identity shown is
published), Tor unavailable. The Tor details dialog says what the last
check found in plain words (not running, SOCKS or control not working,
authentication refused, too old, outdated) and offers a check and the
settings. Nothing in the client offers to connect without Tor. Outside
Tails and Whonix the client dials only once Tor has listed its SOCKS
endpoint (`THREAT_MODEL.md` adversary O): while Tor Browser is closed,
nothing is dialed, and when Tor answers but does not list the configured
SOCKS endpoint, the check says that SOCKS is not working.

A message that cannot be delivered stays Queued; the conversation says
why, from the runtime's last deferral: the contact could not be reached,
Tor is down, the contact has no address yet, the receipt was not confirmed,
or Monolith limits connections. A deferral that Tor reports while Tor is
known to work is said as "could not be reached": an offline contact looks
like that. A failed store is said once, as an error that needs a restart;
a contact that was removed or blocked ends its conversation, which stays
readable and takes no message.

## 6. Workflows

- First start: a profile kept on disk under a passphrase, or a session that
  keeps nothing (P6-7). Later starts ask for the passphrase.
- Identities: made from the identity switcher or the settings, switched
  from the switcher, deleted from the settings after a confirmation that
  says what is removed.
- Contacts: added by pasting a card or an invitation card, or reading one
  from a file (typed path or dropped on the window, at most 64 KiB, checked
  like pasted text). A request to a stranger leaves with the first message
  (P6-6), which the conversation says while the contact waits.
- Requests: listed with the sender's fingerprint, the invitation that
  admitted them, and the name and introduction the sender chose, quoted as
  their words; accepted (naming the contact), declined or blocked.
- Invitations: made with a label only the user sees, shown as a card with
  its QR code, copied or saved, listed with "N of 16 active", revoked with
  or without the requests they admitted.
- Messages: Enter sends, Shift+Enter starts a new line; the composer knows
  the limit of `PROTOCOL.md` section 9; a conversation is read in pages of
  100, newest first, up to 2000 at a time (P6-9).
- Destructive actions (block, remove a contact, delete messages on this
  device, revoke, delete an identity) each have a confirmation that names
  what happens, and are shown done only when the runtime says so.

## 7. Design system

One theme of tokens (`crates/monolith-ui-kit/src/theme.rs`): colours by
role for light and dark, Inter for text and JetBrains Mono for
fingerprints, a spacing scale, radii, strokes, shadows, motion with a
reduced setting, and sizes (window at least 760 by 520 points, 1180 by 760
at first start; sidebar 260 to 420; narrow layout below 880). The
components are those of `crates/monolith-ui-kit/src/lib.rs`; each draws
from the tokens, has its states, and reports its role and label to
accessibility. The screens use components only: a source scan
(`crates/monolith-ui/tests/screens.rs`) fails on a stock egui widget or a
colour, font or size of a screen's own. Screens and components are drawn to
images by a software renderer (`monolith-ui-snapshot`) for review, in
every fixture state, light and dark:
`cargo run -p monolith-ui --features fixtures --example screens -- <dir>`.

## 8. Keyboard

- Tab and Shift+Tab move the focus; the focused control has a ring; Enter
  or Space activates a focused button.
- Enter sends, Shift+Enter starts a new line; Enter submits a dialog's
  field.
- Escape closes a dialog that is not waiting for its action, and clears the
  filter.
- Ctrl+F (Cmd+F) moves to the filter of the list; Alt+Up and Alt+Down show
  the conversation before or after, with the focus in its composer.
- Ctrl+Plus and Ctrl+Minus scale the window (egui's zoom); the scale is not
  kept.

No shortcut of the system is taken over.

## 9. Accessibility

What there is: keyboard use of every screen; a visible focus ring;
contrast by the formula of WCAG 2.1 of at least 7 to 1 for main text and
4.5 to 1 for supporting text, hints, times and text on the accent, on
everything they are drawn on, in both themes
(`crates/monolith-ui-kit/tests/contrast.rs`); status said by an icon and a
word, never by colour alone (Queued, Delivered, the Tor state, ended
conversations); reduced motion; the zoom of section 8; AccessKit roles and
names for the components (buttons, rows, fields with their labels, the
composer and the filter with their placeholder, toggles, segmented
controls, the switcher, menus, banners, message bubbles with their status,
the QR code), on Windows through UI Automation and on Linux through
AT-SPI.

What is missing, and not claimed:

- The client has not been tested with a screen reader (NVDA, Narrator,
  Orca). AccessKit reports what egui knows; text drawn by hand (message
  bubbles, the QR code) has names, but the order a reader follows has not
  been checked.
- Right-to-left scripts are drawn in logical order, unshaped, and scripts
  that need shaping (Arabic, Indic) are drawn without it (`adr/0010`).
- The text size does not follow the system's setting; the zoom is not kept.
- There is no high-contrast theme.

## 10. Clipboard and files

Written to the clipboard only on an explicit action: a card or invitation
card (Copy card), a fingerprint (Copy fingerprint), a message (Copy text in
its menu), or text the user selected (cards and fingerprints are
selectable). The clipboard is read only by a paste into a field. The client
keeps no copy of what it copied; what stays on the system clipboard stays
until something else replaces it, as with any program.

Files: the vault and the message store (the runtime's, `STORAGE.md`); the
settings file (window size, theme, reduced motion, Tor endpoints; 0600 on
Unix); a card saved on request, as a new file in the Downloads folder
(0600 on Unix, never replacing a file); fonts read from fixed system paths.
Nothing else: no log, no cache, no crash report, no eframe persistence.

## 11. No network in the client

The client adds no network path. The UI crates have no network dependency;
the desktop crate talks to the network only through the runtime, whose
peers are reached through `TorBackend` alone (`SECURITY_INVARIANTS.md`).
There is no update check, telemetry, analytics, crash upload, font or image
download, link preview or QR service. eframe always compiles the opening of
URLs in a browser (`adr/0010`): the window removes every such request from
each frame before eframe sees it, which a test and mutation fault UI16
hold. Text that looks like a link is text.

## 12. Performance

Measured on the development machine (Windows 11, release build):

- idle: no frame at all when nothing happens (a test checks every screen,
  `crates/monolith-ui/tests/idle.rs`); the text cursor blinks twice a
  second while a field has the focus; 0 ms of CPU over 10 seconds on the
  first-start screen;
- memory: 117 MB working set with the fallback fonts of Windows loaded
  once (84 MB without them); the GL driver takes most of the rest;
- start: about 0.2 s of CPU to the first frame;
- frames (layout and tessellation, `cargo run --release -p monolith-ui
  --features fixtures --example measure`): 0.1 ms for the first-start
  screen, 0.3 ms for a conversation of 8 messages, 0.5 ms with 100, 1.9 ms
  with 500 and 7 ms with 2000 (the most a window holds); scrolling 2000 at
  7.6 ms a frame; switching identity 2.7 ms; a burst of 1000 events is
  folded into 2 reads in 0.03 ms.

A conversation lays out all the messages of its window each frame it is
drawn; frames happen only on input or events, and the window holds at most
2000 messages, so this stays bounded.

## 13. Limitations

- The name the user gave a contact is not kept once it is blocked or
  removed: the contact store keeps no record of it, and the conversation
  that stays is shown as "Former contact" and the first group of the
  fingerprint (P6-14).
- QR codes are shown, never scanned (P6-11).
- The window's size and maximized state are kept, not its position.
- On Windows the permissions of the profile folder are not checked
  (`STORAGE.md` section 3.8).
- See also section 9.
