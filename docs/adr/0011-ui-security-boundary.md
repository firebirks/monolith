# ADR 0011: The security boundary of the desktop client

Status: decided for Phase 6.
Date: 2026-10-14.

## Context

The runtime of Phase 5 is the authority over identities, contacts,
messages, sessions and Tor. A front end must be able to show and change
all of that without holding any of the objects that carry that
authority, and without becoming a second source of truth: an immediate
mode GUI redraws from whatever state it keeps, and a stale or mixed-up
piece of that state would be shown, or acted on, as if it were true.

## Decision

### Crates

    monolith-runtime        (Phase 5) commands, queries, events
            |
    monolith-desktop        the binary: eframe, the bridge, local files
            |
    monolith-ui             screens and the application shell
       |          |
    monolith-ui-model   monolith-ui-kit
    (state, no egui)    (design system on egui)

- `monolith-ui-model`, `monolith-ui-kit` and `monolith-ui` depend on no
  Monolith crate. They cannot name a `LocalIdentity`, a `Link`, a
  session, a key, a capability, a vault or anything of `monolith-core`:
  the compiler refuses it, and `tests/boundary.rs` of `monolith-desktop`
  checks the dependency lists.
- `monolith-desktop` is the only GUI crate that depends on
  `monolith-runtime`, and it uses `Messenger` and the plain values of
  `monolith_runtime::api` only. Its bridge turns those values into the
  handles of the UI model and back.

### Handles

The UI model names everything by handles of its own: `UiIdentityId`,
`UiContactId`, `UiMessageId`, `UiRequestId`, `UiInvitationId`. They are
presentation handles, not authority: holding one allows nothing; a
command that carries one is checked by the runtime like any other.

- An identity handle holds the durable `IdentityId`; a contact handle
  holds the identity handle and the durable `ContactId`; a message handle
  holds the contact handle and the place of the message in its
  conversation. A record made again, or an identity restored, has new
  identifiers, so a handle of the old one never names the new one.
- Requests and invitations have identifiers in the core that are local to
  one instance of an identity and not kept across restarts. The bridge
  gives each one it reports a new `u64` handle, never reused in the
  process, together with the identity handle, and keeps the mapping.
- No handle is derived from a list position, a display name or the order
  of a loop. egui widget ids are derived from handles.

### State

- Authoritative state (identities, contacts, conversations, messages and
  their status, requests, invitations, the state of Tor) is only ever
  what a query of the runtime returned. The UI model keeps a copy per
  identity and never changes it on its own.
- Events are notifications (ADR 0007): an event makes the model ask
  again for the part of the state it names, for the identity it names.
  An event for an identity other than the one shown changes nothing that
  is shown. A consumer that fell behind (`Missed::Lagged`) asks for
  everything again. The model is correct without any event: starting late
  or missing events only delays an update.
- `Delivered` is shown only when the runtime says so. Pressing Send, a
  command that returned, or a session that exists is never shown as
  delivery.
- Transient state (the selection, the open dialog, drafts, the filter,
  focus) belongs to the UI. A selection is a handle; a dialog that
  confirms an action captures the handles it acts on when it opens, and
  the action uses those, never "the current selection".
- Every command carries the handles it acts on. A result comes back with
  the handles of its command and is applied to them, never to whatever is
  selected at that moment.

### The bridge

- The runtime runs on a tokio runtime of its own thread. The egui frame
  never waits: it takes what arrived from the bridge through a bounded
  channel and sends commands through another, and never blocks on disk,
  Tor, the vault or an acknowledgement.
- The bridge runs queries and forwards events in one task, in order, so
  that the model sees them in an order that the runtime went through.
  Commands that take time (anything that writes) run as tasks of their
  own and report back.

### Secrets and diagnostics

- The vault passphrase is the only secret the GUI handles. It is kept in
  a buffer that is wiped when it is handed to the bridge or the dialog
  closes; the text field that edits it keeps no undo history in egui's
  memory, and its state is removed when the dialog closes. While it has
  the focus the window turns the input method of the system off, so that
  the passphrase is not composed through it; it takes at most 1024
  characters. egui still copies the text of a field while it draws it;
  those copies are not wiped.
- The composer keeps no undo history either: a sent or deleted message
  cannot come back with Ctrl+Z, and message text stays out of egui's
  memory.
- Nothing secret, and no message or peer text, goes into egui's memory,
  widget ids, debug output, logs or the settings file.
- Errors of the runtime are mapped to fixed messages for the user; no
  `Debug` or `Display` of an error reaches the screen.

### Outside the process

- The GUI opens no socket and starts no process. It loads no font, image
  or other resource from anywhere but the binary, except fallback fonts
  from a fixed list of system paths (ADR 0012). It opens no URL: text that
  looks like a URL is text (ADR 0010).
- The clipboard is written only on an explicit action: a public card, an
  invitation card, a fingerprint, or text the user selected. It is never
  read except on an explicit paste.
- Files: a card or an invitation is saved only when the user asks, as a
  new file in the Downloads folder that never replaces one, with
  permissions 0600 (P6-15); nothing else is exported. A card is read from
  a regular file the user names or drops on the window, at most 64 KiB,
  and goes through the same checks as pasted text.
- The settings file (`$XDG_CONFIG_HOME/monolith/desktop.conf`, 0600; on
  Windows `%LOCALAPPDATA%\Monolith\desktop.conf`; on Tails only in the
  Persistent Storage, P6-16) holds the window size, the theme choice,
  reduced motion and the Tor endpoints; nothing else. A value is one line
  without control characters; a line that does not parse leaves the
  default.

## Consequences

- The UI cannot act with an identity's authority even by mistake, and a
  late event or result cannot cross from one identity, contact record,
  request or invitation to another, because they are named by handles
  that differ.
- Every change on screen costs a query. The queries read memory and are
  cheap; the bridge coalesces them per frame.

## Alternatives considered

- Passing `monolith_runtime::api` values into the UI crates directly:
  the identifiers are already plain values, but the UI crates would then
  depend on `monolith-runtime`, and through it on the core.
- Reacting to events by changing the shown state directly: faster, but
  the view would then depend on seeing every event in order, which ADR
  0007 does not promise.
