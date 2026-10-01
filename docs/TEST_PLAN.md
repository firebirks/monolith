# Test plan

Status: plan. Phase 0 has tests for what exists: the limits table, the
message type gate, the session state table, endpoint epochs, the storage
and peer state mappings, and redaction of secret types.

## 1. Principles

- Hostile input is the normal case. Every decoder is tested mostly with
  input it must reject.
- A security-sensitive change arrives with its tests (ARCHITECTURE.md
  section 9).
- Protocol code is sans-IO, so the same decoder runs under unit tests,
  property tests and fuzzers without a network.
- CI never uses the public Tor network.
- Tests that need reduced limits get them through test-only constructors,
  not by changing `limits.rs`.

## 2. Levels and tools

| Level | Tool | Where it runs |
| --- | --- | --- |
| Unit | `cargo test` | CI |
| Property | `proptest` | CI |
| Fuzz | `cargo fuzz` (libFuzzer), nightly toolchain | scheduled, and locally |
| Integration | `MockTorBackend`, scripted SOCKS and control servers | CI |
| Private Tor network | Chutney with an onion service network | local, opt-in |
| Platform | Tails and Whonix virtual machines | manual, per release |
| Public Tor | two nodes over the real network | manual |

## 3. Unit tests

Frames (T-FRAME)

1. Length below the minimum is rejected before any read.
2. Length above the maximum is rejected before any read.
3. Length not congruent to 16 modulo 1024 is rejected.
4. In `IdentityAuth` and `AuthenticatedUnknown`, any length other than 1040
   is rejected.
5. Padding longer than necessary, or non-zero, is rejected.
6. `body_length` larger than the plaintext is rejected.

Fields (T-FIELD): for every variable field, length at maximum is accepted
and maximum plus one is rejected; for every fixed-size message, body length
off by one in each direction is rejected; trailing bytes are rejected;
presence bytes other than 0 and 1 are rejected.

State machine (T-PROTO-STATE): every message type in every state; no
message triggers a dial; no message from an unknown peer changes the
contact store.

Identity (T-ID)

1. An endpoint that does not prove the pinned identity, at handshake
   message 2 or in its proof, is a hard failure and raises the mismatch
   event.
2. No code path replaces a pinned key.
3. Two contacts with identical display names remain distinct.

Contact cards (T-CARD): valid cards of both sizes; every single-bit flip is
rejected; reserved flag bits; epoch zero; wrong version; trailing bytes;
invalid curve points; small-order keys; onion keys with a torsion
component; text form in lower and mixed case, with whitespace, with a
wrong prefix, with non-zero trailing bits, over length.

Endpoint epochs (T-ENDPOINT)

1. Greater epoch from the pinned identity is accepted as pending; the same
   epoch with the same endpoint is a no-op; the same epoch with another
   endpoint is reported as an anomaly; a lower epoch is ignored; a card
   signed by another key is rejected; the pinned epoch never decreases.

Text (T-TEXT): invalid UTF-8; each forbidden code point class per field;
bidirectional controls in names and filenames; zero-width characters;
maximum scalar count for names; NFC normalization of names.

Filenames (T-FILE-NAME): separators, `.` and `..`, control characters,
drive letters, UNC prefixes, reserved device names, leading and trailing
dots and spaces, names that sanitize to nothing.

File transfer (T-FILE)

1. A chunk for a transfer that was not accepted closes the session.
2. Nothing is written before the user accepts.
3. Wrong chunk size, more bytes than declared, completion before all bytes,
   digest mismatch, duplicate transfer identifier.
4. Existing destination file is not overwritten.
5. Messages that cross an abort, a reject or an offer timeout are
   discarded. A file message for an identifier never seen on the session
   is a violation.

Contacts (T-CONTACT)

1. A flood of requests leaves the contact store byte-identical and the
   pending queue at its bound.
2. Crossing requests produce one accepted contact on each side.
3. Deleting a contact and receiving a request with the same name yields an
   unknown identity.
4. Confirmation: when one side holds the other as accepted and the other
   side does not (deleted, restored from backup, never accepted), the
   session is not confirmed and no contact-only message is sent in either
   direction.
5. A side whose record says "requested" becomes "accepted" on ContactAccept
   and the session is confirmed without a reconnect.

Blocking (T-BLOCK)

1. A blocked identity produces no event and no stored state, and receives
   exactly what a stranger with a bad invitation receives.
2. Blocking an accepted contact ends its session; its later sessions are
   never confirmed.

Duplicate sessions (T-DUP)

1. Simultaneous dial in both directions: both sides keep the same session.
2. One session completes much earlier than the other.
3. Same-direction reconnect while the old session is half-open.
4. Reconnect storm from one contact is rate limited.
5. Unacknowledged messages move to the surviving session exactly once.
6. An unauthenticated stream cannot affect an existing session.
7. The preferred session is half-open: the probe gets no Pong and the newer
   session is kept.

Randomness (T-RNG)

1. A failing CSPRNG source makes key generation fail; there is no fallback.

Tor control (T-CTRL)

1. With a recording mock control port, a full run emits only the lines in
   TOR_CONTROL_SURFACE.md.
2. Oversized lines, too many lines, `510` replies, a closed connection in
   the middle of a reply.

SOCKS (T-SOCKS)

1. The request carries address type 0x03 and the expected name.
2. Refused method, error replies, oversized replies, stalls.

Logging (T-LOG)

1. The hostile-peer suite runs at maximum verbosity; the output contains
   none of the secrets and none of the peer-supplied strings used.

## 4. Property tests

- Encode then decode is the identity for every message type.
- Decode then encode reproduces the input bytes exactly for every accepted
  input (canonical form).
- Arbitrary bytes never decode to a value that violates a field limit.
- Arbitrary sequences of valid messages never drive the state machine into
  a state that accepts application data without authentication.
- Arbitrary length prefixes never cause an allocation larger than one
  frame.
- The duplicate-session rule gives the same winner on both sides for any
  pair of identity keys and any interleaving.
- Filename sanitization output never contains a separator and is never
  empty.

## 5. Fuzzing

Targets (planned, `fuzz/`):

| Target | Input |
| --- | --- |
| `preamble` | bytes |
| `handshake_record` | bytes for each of the three messages |
| `frame_decoder` | byte stream split at arbitrary points |
| `message_decoder` | plaintext frame, with each session state |
| `contact_card` | binary card |
| `contact_card_text` | text card |
| `endpoint_update` | message body |
| `text_validation` | bytes, per field kind |
| `filename` | bytes |
| `socks_reply` | bytes |
| `control_reply` | byte stream |
| `vault_header` | bytes |
| `session_sequence` | sequence of decrypted messages against a session |

The property every target enforces:

For arbitrary bytes from a peer, Monolith either rejects them or parses
them into a valid bounded message, without a panic, without running out of
memory, without an allocation larger than one frame, without an unbounded
loop, without an unbounded write, without leaving the designated
directory, and without changing persistent state before authentication and
validation.

Harness rules: an allocation cap well below the default (`-malloc_limit_mb`),
a per-input timeout, and for stateful targets an in-memory store whose
contents are compared before and after for unauthenticated input. A corpus
is kept in the repository once it exists. Crashes become regression tests.

## 6. Hostile peer suite (T-MAL)

Run against a real session over `MockTorBackend`:

- endless input without completing a handshake record or a frame;
- frames at maximum length and at maximum plus one;
- length prefixes of 0, 1, 65535;
- truncated handshake and truncated frames;
- duplicated and reordered frames;
- random bytes as frames;
- messages replayed across sessions;
- invalid UTF-8, large combining sequences, bidirectional controls;
- a flood of contact requests from many identities;
- stalling at every step of authentication;
- a flood of file offers;
- traversal and device names as filenames;
- wrong file size, too many chunks, premature completion;
- stale endpoint epoch;
- a proof for another session, a proof with the wrong role, a proof by the
  wrong key;
- an identity mismatch on an outbound session.

Each case asserts the outcome (session closed or input ignored), that no
event reached the front end unless specified, and that budgets held.

Conforming peers under load, in the same harness: a contact that sends
faster than a per-contact rate is slowed and not disconnected; a full
outbound queue resent after a reconnect arrives completely; a profile sent
on each of several quick reconnects does not end the session.

Confirmation tests (T-CONFIRM)

1. An unknown peer's view (bytes received and time to close) is the same
   whether its request was queued, dropped for a bad invitation, dropped as
   a duplicate, dropped because the sender is blocked, or dropped because
   the queue was full, and whether or not contact sessions exist.
2. No message type contains a field for a third party's identity or
   address (checked against the message table).

## 7. Concurrency

- Simultaneous connections in both directions.
- Shutdown while connecting, during a handshake, during a transfer.
- Tor disappears: SOCKS refuses, control connection closes.
- The control connection drops and returns; the service is re-created.
- A SOCKS connection stalls forever.
- The message store is unavailable; the vault is locked by another process.
- Cancellation of every kind of task leaves no task behind.

## 8. Resources (T-RES)

1. Memory high-water mark under the hostile suite stays under the computed
   ceiling of RESOURCE_LIMITS.md section 11.
2. Task count stays under its bound under a connection flood.
3. Every channel is bounded: lint, plus a test that fills each queue and
   observes backpressure.
4. File descriptors stay under their bound under a connection flood.
5. Notifications and pending requests stay at their bounds under a request
   flood.

## 9. Tails matrix (T-PLAT-TAILS, manual)

Per release, on the current Tails:

| Case | Expectation |
| --- | --- |
| Fresh boot, no Persistent Storage | runs ephemeral; `EPHEMERAL SESSION` shown; nothing written |
| Persistent Storage present but locked | as above; creating a persistent identity is refused with an explanation |
| Persistent Storage unlocked | persistent identity can be created under `~/Persistent`; survives reboot |
| Tor not yet bootstrapped | "Tor connecting"; no traffic attempted |
| Tor disconnect and reconnect | service removed and re-created; sessions resume |
| Profile installed | service publishes through port 951 |
| Profile missing | clear error naming the filtered command; no crash |
| `ADD_ONION` under `Sandbox 1` | works (open item T1) |
| Two Monolith peers | chat both ways |
| File transfer | offer, accept, complete; reject; abort |
| Restart of the application | ephemeral identity gone; persistent identity back |
| Power off and reboot | no trace outside Persistent Storage |
| Sockets held | only SOCKS 9050, filter 951 and the listener |
| Firewall reject log | no entry caused by Monolith |
| One tor process | yes |

## 10. Whonix matrix (T-PLAT-WHONIX, manual)

Per release, on the current Whonix, VirtualBox or KVM, and Qubes-Whonix:

| Case | Expectation |
| --- | --- |
| Gateway and Workstation, profile enabled, port opened | service publishes; chat works |
| Profile not enabled | clear error naming the filtered command |
| Firewall port not opened | service publishes but no inbound session; `doctor` reports it |
| Gateway Tor unavailable | "Tor disconnected"; nothing sent |
| Workstation restart | persistent identity back; service re-created |
| Gateway restart | service re-created after reconnect |
| SOCKS isolation | streams of two contacts use different circuits (checked on the Gateway) |
| Unauthorized control commands | `SETCONF`, `GETINFO address` and other commands outside the merged profile are refused by the Gateway |
| Listener exposure | bound to the Workstation's internal address only; not on any other interface |
| Second Workstation connects to the listener directly | treated as an unknown peer; cannot pass authentication |
| One Tor | no tor process in the Workstation |
| Qubes-Whonix | control address, listener address and firewall drop-in work with dynamic addresses |

## 11. Network privacy (T-NET)

1. In a Linux network namespace in which only Tor's SOCKS and control
   endpoints are reachable: Monolith works. With Tor stopped: every network
   operation fails,
   and a packet capture on the namespace shows no DNS query, no direct TCP
   connection, no HTTP and no other traffic.
2. With the SOCKS endpoint configured but refusing connections, dials fail
   and are retried on schedule; nothing else is attempted.

An idle run produces no traffic other than keepalives on open sessions and
the reconnect schedule.

On Tails this cannot be shown by blocking, because Tails redirects direct
TCP into Tor. The namespace test on a development machine is the proof; the
Tails matrix checks sockets and the firewall log.

## 12. Injection (T-INJ)

Through every peer-supplied text field (display name, profile text,
introduction, chat text, filename): newline, the field separators of every
storage record, SQL metacharacters, JSON and TOML syntax, path separators
and traversal, terminal escape sequences, log format directives. Assert
that the stored value is byte-identical to the validated input, that no
other record changed, that CLI output is escaped, and that log output is
unaffected.

## 13. Crash safety (T-CRASH)

A fault-injecting file layer stops a write at each step (create, partial
write, before sync, before rename, before directory sync) during: identity
creation, storing the Onion Service key, contact acceptance, endpoint
update, queue update, file completion. After restart: no partial contact,
no epoch lower than before, no new identity where one existed, the vault
opens.

## 14. CI

On every change:

    cargo fmt --all --check
    cargo clippy --workspace --all-targets --locked -- -D warnings
    cargo test --workspace --locked
    cargo +1.85 check --workspace --all-targets --locked
    cargo deny check
    cargo audit

Scheduled: fuzzing of every target for a fixed time, with the corpus
cached. Dependency updates are reviewed by a person; nothing is merged
automatically.

## 15. Exit criteria

| Phase | Before it is called done |
| --- | --- |
| 1 | T-FRAME, T-FIELD, T-CARD, T-TEXT, T-FILE-NAME, T-PROTO-STATE pass; property tests in place; fuzz targets for every decoder run clean for a fixed budget |
| 2 | handshake test vectors committed; T-MAL handshake cases pass; proofs fail in every relay and replay case |
| 3 | T-SOCKS, T-CTRL pass; T-NET-1 passes; two-node chat over a private Tor network |
| 4 | T-CONTACT, T-ID, T-DUP, T-CONFIRM, T-CRASH, T-INJ pass |
| 5 | T-FILE passes; transfer fuzzing clean |
| 6, 7 | platform matrix passes on the current release |
| 9 | T-RES passes with measurements recorded |
