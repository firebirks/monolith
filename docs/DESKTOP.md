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
names; many events about one part make one read. Two events carry a value
of the runtime that the model keeps as it came: the state of an identity's
Onion Service, which the next read of the identity returns too, and a
deferred delivery, whose reason and time of the next attempt no query
returns. After missed events everything is read again and Tor is checked
again. Transient
state belongs to the window: the selection (a handle), the open dialog
(with the handles it captured when it opened), drafts, the filter, focus.

### Queued and Delivered

A message is shown with exactly the status the runtime gives it: Queued
(a clock), Delivered (two ticks), or received (no mark). Delivered comes
only from the runtime's answer for that message, which follows an
acknowledgement made durable by the contact (`adr/0007-delivery-semantics.md`).
Pressing Send, the command returning, or a session existing is never shown
as delivery. The draft stays editable while the message is being queued,
and loses the text sent only once the runtime says the message is durably
queued, and only if the draft has started with that text ever since it was
sent: what was typed after it stays, and a draft cleared or changed within
the text sent meanwhile stays whole, also when it starts with that text
again. A refused send keeps the draft, with what was typed meanwhile, and
says why.

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
SOCKS endpoint, the check says that SOCKS is not working. Step 21 of the
private Tor network test (`tests/tor-network/README.md`) runs two clients
with this configuration against real Tor processes, driven through their
window code by `crates/monolith-desktop/examples/tor_smoke.rs`.

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
once nothing is queued. While Tor is unavailable the notice says that
nothing can be delivered: messages are still queued. A failed store is said
once, by a banner across the top of every view that asks for a restart; the
command that met it adds no error or notice of its own, and nothing can be
sent after it. A contact that was removed or blocked ends its conversation,
which stays readable, says once that it ended, and has no composer.

The window's title counts what waits over all identities (messages not
read and requests not answered), as "Monolith (7)", and the window asks
for attention when that count grows while it is in the background; the
identity switcher gives each identity its own counts. The title, count
included, is visible to anything that reads window titles: the task bar,
window switchers, screen recorders and other programs on the same desktop.

## 6. Workflows

- First start: a profile kept on disk under a passphrase, or a session that
  keeps nothing (P6-7). Later starts ask for the passphrase. A passphrase
  field takes at most 256 characters, which are at most 1024 bytes of
  UTF-8, the most the vault takes, and its buffer has room for that from
  the start, so that typing or pasting never moves it. A failure of the
  passphrase is said under its field, which it marks; the first start says
  to use a long phrase of several words, and that a lost passphrase cannot
  be recovered.
- Identities: made from the identity switcher or the settings, switched
  from the switcher, deleted from the settings after a confirmation that
  says what is removed. The card of the identity shown is one click away:
  "Share my card" in the switcher, the card button in the footer of the
  list, and the empty state of an identity without contacts.
- Contacts: added by pasting a card or an invitation card, or reading one
  from a file (typed path or dropped on the window, at most 64 KiB, checked
  like pasted text). A request to a stranger leaves with the first message
  (P6-6), which the conversation says while the contact waits. The runtime
  says whether the card the contact's record holds after the import is an
  invitation, and the contact keeps saying it (P6-20); for a contact that
  accepted already, a card added changes nothing. After a card that is not
  an invitation, "Add contact" stays open to say that the other side must
  add the user's card too before messages can arrive, with a button to the
  user's card; the conversation says the same, with the same button, until
  the contact accepts. After an invitation it says that the request goes
  out with the messages and should be seen through that invitation (only
  while the invitation is active and the other side takes requests),
  without a name or a message. What such a contact waits for is said in
  the same words under its name, in its conversation and in its details:
  "Request not sent yet" while nothing was written to it, "Waiting for
  them to accept your request" once something was, "Waiting for them to
  add your card" after a card that is not an invitation. The list says
  them under its name while the conversation has no message; once it has
  one, the list shows that message with an hourglass, which says the wait
  under the pointer. Deleting the messages of such a contact says that the
  request goes out only with a message.
- An identity without contacts offers to add one and to share its card;
  Invite is in the strip under the list.
- The user's own card says what a card alone does under the request mode
  of the identity: with Invitations only, someone who has only the card
  can reach the user once the user has added their card too; with Anyone
  with my card, anyone who has it can send a request; with Nobody, no
  request arrives at all. Under Nobody, the invitations of the identity,
  "New invitation" and the card of an invitation say that no request
  arrives through an invitation either.
- Requests: listed with the sender's fingerprint, the invitation that
  admitted them, and the name and introduction the sender chose, quoted as
  their words; accepted (naming the contact), declined or blocked. Accepting
  a request that came through an invitation, while the identity takes
  requests, ends with a toast that says the invitation still admits
  requests, with Revoke, which asks as revoking from the settings does.
  Declining asks first: nothing is sent, the sender's later requests are
  dropped (`PROTOCOL.md` section 12), and the client neither lists nor
  undoes a decline; adding the sender's card takes them off the declined
  list (P4-13).
- Blocking: a contact from the menu of its conversation or its details,
  the sender of a request from the request, each after a confirmation.
  Someone blocked cannot reach the identity: no session, no request, no
  message.
- Unblocking: Settings > Identities lists, under each identity, the people
  it blocked by their short fingerprint, the only thing the contact store
  keeps of them (P6-14), each with Unblock; an identity that blocked nobody
  says so in one line. Unblocking asks first, naming them by the same
  fingerprint, and a toast says what it did.
  Someone unblocked has no record at all (P6-21): not a contact again and
  not declined; a request from them is considered as the request mode
  says, and adding their card makes a new contact with a new conversation.
  Since nothing is kept of them, the client can block them again only from
  a request they send or once their card is added, which the question
  says. A conversation that ended with the block stays ended.
- Invitations: made with a label only the user sees, shown as a card with
  its QR code, copied or saved, listed with "N of 16 active", revoked with
  or without the requests they admitted.
- Messages: Enter sends, Shift+Enter starts a new line; the composer knows
  the limit of `PROTOCOL.md` section 9; a conversation is read in pages of
  100, newest first, up to 2000 at a time (P6-9).
- Destructive actions (block, decline, remove a contact, delete messages
  on this device, revoke, delete an identity) and unblocking each have a
  confirmation that names what happens, with Cancel focused so that Enter
  changes nothing, and are shown done only when the runtime says so.
- Closing: the window's close button, Alt+F4 and a close by the system take
  one path (`window::close_or_ask`; the client has no Quit shortcut of its
  own). When closing costs nothing the window is hidden at once and the
  runtime stops behind it; on Wayland, where a window cannot hide itself,
  it goes when eframe destroys it, since the runtime and the file thread
  are waited for only after that (`Window::start`). Otherwise the close is
  cancelled, the window stays (brought back if it was minimized) and asks
  first: in a session that saves nothing, that closing deletes every
  identity, contact and message for good, with a danger button; and how
  many messages are not delivered yet, which a profile on disk tries again
  when it next opens and a session that saves nothing never delivers. Close
  Monolith closes the window, hidden only then; Cancel, Escape, Enter
  (Cancel has the focus) or a click beside the question keep it, and the
  next close asks again. The question takes the place of an open dialog,
  which is back if the window stays.

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
every fixture state, light and dark, at 1180 by 760, 820 by 600 and 760 by
520 points, a folder for each size:
`cargo run -p monolith-ui --features fixtures --example screens -- <dir>`
(`-- <dir> <name> <width> <height>` draws one state at one size). The
states include every dialog (the fixtures named `dialog-`), toasts over a
conversation with notices and over a dialog, and the empty and ended
views.

## 8. Keyboard

- Tab and Shift+Tab move the focus; the focused control has a ring; Enter
  or Space activates a focused button. Tab stops on controls only: the
  edge of the sidebar and the scroll bars are dragged with the pointer and
  take no focus, and lists scroll with the wheel or as the focus moves.
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

What there is: keyboard use of every screen, but for the menu of a
message (below); a visible focus ring;
contrast by the formula of WCAG 2.1 of at least 7 to 1 for main text and
4.5 to 1 for supporting text, hints, times and text on the accent, on
everything they are drawn on, in both themes
(`crates/monolith-ui-kit/tests/contrast.rs`), and wells, fields and cards
told from what they are drawn on by their fill or a hairline; status never
by colour alone: the Tor state is a word beside its dot, the status of a
message (Queued, Delivered) and the markers of the list of conversations
(waiting for acceptance, messages queued, no longer a contact) are glyphs
that say their word as a tooltip and to a screen reader, and an ended
conversation says so in words in its bar and its notice; reduced motion;
the zoom of section 8; AccessKit roles and
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
- A conversation lays out only the messages in view and half a view
  around it (section 12), so only those are in the AccessKit tree; a
  screen reader reaches the others by scrolling. The keyboard does not
  reach the "Show more" of a message out of view either.
- The menu of a message (Copy text) opens with the secondary button of a
  pointer only; the keyboard cannot open it. Selecting the text of a
  message takes a pointer too; Ctrl+C then copies it.
- While a passphrase field has the focus the input method of the system is
  off (`adr/0011`): a passphrase that can only be typed through an input
  method can only be pasted.
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
settings file (window size, theme, reduced motion, Tor endpoints; not
encrypted, apart from the profile, as the Data page of the settings says;
0600 on Unix); a card saved on request, as a new file in the Downloads folder
(0600 on Unix, never replacing a file); fonts read from fixed system paths.
Nothing else: no log, no cache, no crash report, no eframe persistence.

A card file is read only when the user names it or drops it on the window,
only if it is a regular file, and at most 64 KiB of it. The name is looked
at (`lstat`) before anything is opened, and anything but a regular file is
refused unopened, a symbolic link included: opening a device can do
something of its own, such as rewinding a tape, and a pipe or a device can
block or never end. On Unix the file is then opened without following a
link and without waiting (`O_NOFOLLOW | O_NONBLOCK`) and read only if it is
the file looked at, the same device and inode (`fstat`); a name given to
something else between the two steps is refused unread. What is put there
in that moment, by someone who can write to the folder, is still opened
before it is refused. On Windows its path must be an absolute path on a
drive (`C:\...`): a share (`\\server\...`), a device path (`\\?\`, `\\.\`)
or a relative path is refused, so that naming a share does not make
Windows reach another computer over SMB, outside Tor; the file opened is
checked again to be a regular file. A path on a drive can still reach the
network, and the client cannot tell: a drive letter mapped to a share, a
link or junction to a share, and a file of a cloud folder kept online only
(downloaded when it is read) are read over the network, outside Tor, as
Windows reads them.

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
  screen, 0.2 ms for a conversation of 8 messages, and 0.2 ms with 2000
  (the most a window holds) whether they are of 42 characters, of 1.5 KB
  or of 16 KB (the largest a message can be); the frame that opens such a
  conversation 0.7 to 1.3 ms; scrolling through them 0.2 ms a frame, at
  most 1.3 ms when a message of 16 KB comes into view; a sidebar of 500
  contacts whose newest messages are of 16 KB 0.4 ms, 0.5 ms scrolling;
  switching identity 0.3 ms; a burst of 1000 events is folded into 2 reads
  in 0.03 ms.

A conversation lays out and paints only the messages in view and half a
view above and below it. The height of a message at the width it is shown
in is measured the first time it is laid out and kept; until then it is
estimated from the length of its text, and the space of the messages out
of view is that of their heights. The view keeps the message at its top
where it is when heights above it change (older messages read, a message
measured), and stays at the bottom as messages arrive while it is there.
The text of a message is made ready to draw (`present::prepare`) once, the
first time it is drawn, and kept with the message in the UI model. The rows
of the sidebar are made when what they show changes (a read of the
identity, an unread count), not each frame, and a preview reads only the
start of the newest message.

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
  (P6-20). Nor can it know whether a request sent through an invitation
  was seen: only that it waits to be accepted.
- Unread counts are kept in memory only: they start again from nothing
  each time Monolith opens. The Data page of the settings says so.
- Declined requests are not listed, and a decline cannot be undone in the
  client but by adding the sender's card. Someone unblocked can be blocked
  again only from a request of theirs or once their card is added: the
  contact store keeps no record of them (P6-21).
- The window's size and maximized state are kept, not its position.
- On Windows the permissions of the profile folder are not checked
  (`STORAGE.md` section 3.8).
- See also section 9.
