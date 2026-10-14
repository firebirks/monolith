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
(contacts, conversations, requests, invitations, the people it blocked,
the state of its service, its request mode) is one run of queries with
nothing in between.

Every command carries the handles it acts on (`UiIdentityId`,
`UiContactId`, `UiMessageId`, `UiRequestId`, `UiInvitationId`,
`UiBlockedId`), and its answer names the command. Identity, contact and
message handles hold the durable identifiers of the runtime. Requests,
invitations and blocked people, whose references in the core are local to
one instance of an identity, get a number from the bridge, never given
twice in a process; the bridge checks on every use that the number belongs
to the identity the command names, so a number of one identity names
nothing of another. A number is forgotten once a read of its identity no
longer lists what it names: someone unblocked and, after such a read,
blocked again gets a new number, and the old one names nothing.

Errors of the runtime are mapped to one of the fixed sentences of
`monolith_ui_model::Failure` (`crates/monolith-desktop/src/convert.rs`).
No `Debug` or `Display` of an error, and no message, name or card, reaches
the screen, a log or the settings file.

## 4. What is shown, and from where

Everything authoritative on screen (identities, contacts, conversations,
messages and their status, requests, invitations, the people blocked, the
state of Tor) is what a query of the runtime answered, kept per identity.
The model never changes it on its own. Events are notifications: one makes
the model read again the part of the state it names, for the identity it
names; many events about one part make one read. Transient state belongs to the window:
the selection (a handle), the open dialog (with the handles it captured when
it opened), drafts, the filter, focus.

### Queued and Delivered

A message is shown with exactly the status the runtime gives it: Queued
(a clock), Delivered (two ticks), or received (no mark). Delivered comes
only from the runtime's answer for that message, which follows an
acknowledgement made durable by the contact (`adr/0007-delivery-semantics.md`).
Pressing Send, the command returning, or a session existing is never shown
as delivery. The draft stays editable while the message is being queued,
and loses the text sent only once the runtime says the message is durably
queued: what was typed after it stays, and a draft changed within the text
sent stays whole. A refused send keeps the draft, with what was typed
meanwhile, and says why.

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

The top bar shows one of four states (`TorIndicator`): Checking Tor (at
start), Connecting to Tor (Tor answers but builds a circuit, or the service
of the identity is being published), Tor connected (the service of the
identity shown is published), Tor unavailable. The narrow layout, which has
no top bar while the list is shown, shows the same state beside the
identity switcher. The labels name Tor, not the contact: whether a contact
is online is never known. The Tor details dialog says what the last
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
like that. The same notice says, once, that messages are delivered only
while Monolith is open on both sides (the runtime dials only while it runs,
P5-8), and, from the delay of the last deferral, when the next attempt is
due ("Next try in about 4 minutes", "in less than a minute"; nothing once
it is due, and nothing while Tor is down). The model keeps that delay as
the time it ends, counted from when the window took the event, and drops it
once nothing is queued. A failed store is said once, by a banner across the
top of every view that asks for a restart; nothing can be sent after it. A
contact that was removed or blocked ends its conversation, which stays
readable and takes no message.

The window's title counts what waits over all identities (messages not
read and requests not answered), as "Monolith (7)", and the window asks
for attention when that count grows while it is in the background; the
identity switcher gives each identity its own counts.

## 6. Workflows

- First start: a profile kept on disk under a passphrase, or a session that
  keeps nothing (P6-7). Later starts ask for the passphrase.
- Identities: made from the identity switcher or the settings, switched
  from the switcher, deleted from the settings after a confirmation that
  says what is removed.
- Contacts: added by pasting a card or an invitation card, or reading one
  from a file (typed path or dropped on the window, at most 64 KiB, checked
  like pasted text). A request to a stranger leaves with the first message
  (P6-6), which the conversation says while the contact waits. The runtime
  says whether the text added was an invitation, and the contact keeps
  saying it (P6-20). After a card that is not an invitation, "Add contact"
  stays open to say that the other side must add the user's card too before
  messages can arrive, with a button to the user's card; the conversation
  says the same, with the same button, until the contact accepts. After an
  invitation it says that the request leaves with the first message and is
  seen through that invitation, without a name or a message.
- The user's own card says what a card alone does under the request mode
  of the identity: with Invitations only, someone who has only the card
  can reach the user once the user has added their card too; with Anyone
  with my card, anyone who has it can send a request; with Nobody, no
  request arrives at all.
- Requests: listed with the sender's fingerprint, the invitation that
  admitted them, and the name and introduction the sender chose, quoted as
  their words; accepted (naming the contact), declined or blocked.
- Blocking: a contact from the menu of its conversation or its details,
  the sender of a request from the request, each after a confirmation.
  Someone blocked cannot reach the identity: no session, no request, no
  message.
- Unblocking: Settings > Identities lists, under each identity, the people
  it blocked by their short fingerprint, the only thing the contact store
  keeps of them (P6-14), each with Unblock; an identity that blocked nobody
  says so in one line. Unblocking takes no confirmation, since blocking
  again undoes it, and a toast says what it did. Someone unblocked has no
  record at all (P6-21): not a contact again and not declined; a request
  from them is considered as the request mode says, and adding their card
  makes a new contact with a new conversation. A conversation that ended
  with the block stays ended.
- Invitations: made with a label only the user sees, shown as a card with
  its QR code, copied or saved, listed with "N of 16 active", revoked with
  or without the requests they admitted.
- Messages: Enter sends, Shift+Enter starts a new line; the composer knows
  the limit of `PROTOCOL.md` section 9; a conversation is read in pages of
  100, newest first, up to 2000 at a time (P6-9).
- Destructive actions (block, remove a contact, delete messages on this
  device, revoke, delete an identity) each have a confirmation that names
  what happens, and are shown done only when the runtime says so.
  Unblocking is not one of them: it is undone by blocking again.
- Closing: the window's close button, Alt+F4 and a close by the system take
  one path (`window::close_or_ask`; the client has no Quit shortcut of its
  own). When closing costs nothing the window is hidden at once and the
  runtime stops behind it. Otherwise the close is cancelled, the window
  stays (brought back if it was minimized) and asks first: in a session
  that saves nothing, that closing deletes every identity, contact and
  message for good, with a danger button; and how many messages are not
  delivered yet, which a profile on disk tries again when it next opens
  and a session that saves nothing never delivers. Close Monolith closes
  the window, hidden only then; Cancel, Escape, Enter (Cancel has the
  focus) or a click beside the question keep it, and the next close asks
  again. The question takes the place of an open dialog, which is back if
  the window stays.

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
- Escape closes a dialog that is not waiting for its action, clears the
  filter, and leaves the settings when no dialog or menu is open.
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
until something else replaces it, as with any program. On Windows, when
the clipboard history is turned on (Windows+V), the system keeps copied
cards and invitation cards in that history, and synchronises it to the
user's other devices if that is turned on too; the client cannot keep a
copy out of it.

Files: the vault and the message store (the runtime's, `STORAGE.md`); the
settings file (window size, theme, reduced motion, Tor endpoints; 0600 on
Unix); a card saved on request, as a new file in the Downloads folder
(0600 on Unix, never replacing a file); fonts read from fixed system paths.
Nothing else: no log, no cache, no crash report, no eframe persistence.

A card file is read only when the user names it or drops it on the window,
only if it is a regular file, and at most 64 KiB of it. On Windows its path
must be an absolute path on a drive (`C:\...`): a share (`\\server\...`),
a device path (`\\?\`, `\\.\`) or a relative path is refused, so that
reading a card never makes Windows reach another computer over SMB, outside
Tor.

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
  second while a field has the focus; a conversation that says when
  delivery is tried next draws again when those words change, at most once
  a minute; 0 ms of CPU over 10 seconds on the first-start screen;
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
  fingerprint (P6-14). The list of blocked people shows fingerprints
  only, for the same reason.
- QR codes are shown, never scanned (P6-11).
- A contact added by a card that is not an invitation is said to wait until
  it adds the user's card. The client cannot know whether that side takes
  requests from anyone, in which case the request is also listed there
  (P6-20). The list of conversations does not tell the two kinds of
  waiting contact apart; the conversation does.
- The window's size and maximized state are kept, not its position.
- On Windows the permissions of the profile folder are not checked
  (`STORAGE.md` section 3.8).
- See also section 9.
