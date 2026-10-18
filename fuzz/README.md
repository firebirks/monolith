# Fuzzing

A `cargo fuzz` project for the protocol core, the session layer, the Tor
adapter's parsers, the vault payload, the contact store, the message
store payload, the message state machine, and the presentation and state
of the desktop client. It is
not a member of the main workspace: it needs a nightly toolchain and has
its own lock file.

Every target calls the same sans-IO code the application uses; there is no
test-only parsing path. The three session targets run real handshakes and
real encryption. So that an input behaves the same way twice, they run the
handshake with fixed ephemeral keys. The functions that allow this exist in
`monolith-session` only when it is compiled with `--cfg fuzzing`, which
`cargo fuzz` sets, and in that crate's own tests.

## Targets

| Target | Input | Property |
| --- | --- | --- |
| `base32` | text | fails, or decodes to bytes that encode back to the same text |
| `contact_card` | bytes | fails, or decodes to a card that encodes to the same bytes and whose signature verifies |
| `contact_card_text` | text | fails, or parses to a card whose canonical text parses to the same card |
| `message_body` | type selector, body | fails, or decodes to a message that encodes to the same bytes |
| `frame_plaintext` | state selector, mode, then a plaintext or the parts of one | fails, or yields a message type legal in that state and a body that re-encode to the input; an undamaged frame is accepted exactly when its type is legal in the state |
| `frame_stream` | state, piece size, mode, then a byte stream or records to build one from | fails, or yields payloads of legal length; never holds more than one frame; undamaged frames all come out again |
| `text_fields` | bytes | every validator accepts exactly what the rules of PROTOCOL.md section 9 accept, written out a second time in the target, and keeps accepted text byte for byte; save names are single safe path components |
| `session_sequence` | progress and direction, standing, events | no application data before confirmation; no backward transition; a card that does not belong to the authenticated peer is always a violation; nothing is sent that `may_send` forbids; identities that are not contacts look alike, a contact with a stale card or a pending key among them |
| `credential_sequence` | events: admission, announcement on a session of a chosen key, confirmation, import, replacement, each with one of 72 cards of one identity | the invariants of the credentials hold and every state is one `Credentials::restore` accepts; the active epoch never goes down; before each event the card has the same relation whatever its source, and evaluating it changes nothing; the active key changes exactly on a promotion, and only by a proven authorized successor, a confirmed pending card or a replacement; a promotion retires the previous key; what changes nothing leaves the state as it was; an announcement on a session of another key changes nothing; the standing is a contact's exactly for the active key or a promoted one |
| `contact_store` | operations of the user on three remote identities: import, confirm a pending card, confirm a dial card, block, unblock, delete, mark verified, each with one of 40 cards of that identity | the store of a local identity returns and holds exactly what a model built on `contact::decide` alone returns and holds; the credentials keep their invariants; a contact's active epoch never goes down while it stays a contact; a promotion retires the previous key; the dial card is always one of the contact |
| `vault_payload` | the plaintext payload of a vault | fails, or decodes to contents within every limit whose encoding, followed by the zero padding, is the input, and which decode again to the same contents |
| `message_store` | the plaintext payload of a message store | fails, or decodes to contents within every limit of `RESOURCE_LIMITS.md` section 5, in increasing order of identity, conversation and sequence, whose encoding with the zero padding is the input and decodes to the same contents; the state loaded from them encodes to the same contents, with the length it accounts for |
| `message_sequence` | operations on two local identities with two contact records each: queue, queue naming another peer, receive, acknowledge, delete a conversation, remove an identity, restart through the encoded payload | the message state holds and returns exactly what a model written in the target holds and returns; the head of every queue is its oldest queued message; the encoding has the length the state accounts for; a delivered message never becomes queued again while its conversation lives |
| `handshake_responder` | record and piece size, mode, then a stream or the damage to apply to the genuine one | a session exists only after the two genuine messages of an initiator; the reply is made only to a first message that verifies; the handshake takes no record and ends the same way under each; the genuine messages are always accepted, and other accepted ones are judged by what they prove, not compared with the stored transcript; afterwards every message has exactly the actions its record (none, blocked, requested or accepted) allows: a stranger or a blocked identity gets a Close and nothing else, a requested contact is marked accepted once, and nothing is delivered before the session is confirmed |
| `handshake_initiator` | piece size, mode, then a stream or the damage to apply to the genuine one | the third message, which carries the initiator's identity, is made only for a second message that Noise accepts, and always for the genuine one; the session is one with the identity that was dialed or there is none; no comparison with the stored transcript |
| `session_frames` | piece size, then operations: send, deliver, deliver part of a frame, change a byte, drop, repeat, inject, close, block, remove the contact, lose the stream, withdraw, let time pass, expire at the next deadline | a message is delivered only if the peer sent it, once and in order; nothing is delivered from a direction that was interfered with, or after the age limit and its grace; a violation occurs only there; from the age limit on nothing but Close is sent; every way of ending a session sends the same Close; a begun frame that does not complete within `FRAME_READ_TIMEOUT` of its first byte, or no complete frame for `IDLE_TIMEOUT`, ends the session without a Close, the age limit with one; a session that expired or failed stays over |
| `tor_control_reply` | piece size, then bytes from a control endpoint | the parser keeps every line, line count and reply size limit, an error is final, every reply has one status code and is never an event; each interpretation (PROTOCOLINFO, AUTHCHALLENGE, GETINFO, ADD_ONION) is consistent, and a private key appears only in the new-key form; 64 arbitrary key bytes never add a line to `ADD_ONION` |
| `socks_reply` | bytes from a SOCKS proxy after a CONNECT | a reply is complete exactly when its address type's bytes have arrived, never longer than `MAX_SOCKS_REPLY_LEN`; shorter prefixes are incomplete, not wrong; an error stays an error whatever follows |
| `ui_present` | selector byte (expand, preview limit, no local name), then text | no panic; no invisible or direction-changing character reaches the output as itself, only as its `<U+XXXX>` mark (the class is written out in the target from `PROTOCOL.md` section 9); a text of at most 16 characters comes out exactly as the rules say; an unexpanded message is a prefix of the expanded one, holds fewer lines than `MAX_RENDERED_MESSAGE_LINES` and is collapsed exactly when lines were left out; a preview is one line within its limit, is the line itself when nothing is cut and ends in an ellipsis when something is; `sendable` agrees with a second statement (not only white space, at most `MAX_MESSAGE_BYTES`); the name helpers return the local name or the fallback built from the first group of the fingerprint |
| `ui_model_sequence` | five-byte steps: user actions (select identity or conversation, dialogs, drafts, send), commands taken, answers to commands with any outcome built from small pools of handles of two identities, every kind of runtime event, stop, start | no panic; the identity shown is a listed one; a selected conversation belongs to its identity; every command taken carries the handles of one identity and a number above the one before; commands and notices stay within a constant times the steps; an answer to a number never issued, or already answered, changes nothing (everything the public accessors of the model show is equal before and after) |

The property all of them share: for arbitrary input the code either rejects
it or produces a valid bounded value, without a panic, without an allocation
sized by the input, and without an unbounded loop.

The text a user imports as a contact or invitation card in the desktop
client (pasted, or read from a file) goes to the same parser as every other
card, so `contact_card_text` covers that boundary and the desktop has no
target of its own for it. The drawing code (egui) is not fuzzed.

Four targets have a second input mode. Random bytes almost never form a
padded frame, and never a handshake message, so in that mode the target
builds valid input and then damages one byte if the input says so. The
first lines of each target file describe its input layout.

## Seeds

`seeds/<target>/` holds valid inputs for each target: signed contact cards
in both forms, one body and one frame of every message type, streams of
frames, text samples, event sequences for the session logic, the genuine
messages of a handshake with encrypted frames behind them, and operation
lists for the frame target. Without them the fuzzer cannot get past a
signature check, a padding check or an authentication tag.

The seeds are generated, not written by hand. Tests build them from
fixed values and fail if a committed seed is not what the current code
produces: `fuzz_seeds` in `monolith-protocol` for the first nine targets,
`seeds` in `monolith-session` for the three session targets, `fuzz_seeds`
in `monolith-tor` for the two Tor targets, `fuzz_seeds` in
`monolith-storage` for `vault_payload` (payloads of version 1 and 2) and
`message_store`, `fuzz_seeds` in `monolith-core` for `contact_store`
and `message_sequence`, and `fuzz_seeds` in `monolith-ui-model` for
`ui_present` and `ui_model_sequence`. After a format change:

    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-protocol --test fuzz_seeds
    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-session seeds
    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-tor --test fuzz_seeds
    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-storage --test fuzz_seeds
    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-core --test fuzz_seeds
    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-ui-model --test fuzz_seeds

The working corpus that a run produces goes to `corpus/<target>/`, which is
not committed.

## Running

    cargo install cargo-fuzz
    cd fuzz
    cargo +nightly fuzz run <target> corpus/<target> seeds/<target> -- -malloc_limit_mb=64 -timeout=5

The first directory is where new inputs are written; the second is read
only. Add `-max_total_time=<seconds>` for a bounded run. The allocation
limit is deliberately far below the default, so that an allocation sized by
input shows up as a failure.

On Windows the address sanitizer runtime that ships with the MSVC toolchain
(`clang_rt.asan_dynamic-x86_64.dll`) has to be on `PATH`.

To check that the targets compile without running them:

    RUSTFLAGS="--cfg fuzzing" cargo +nightly check --bins

## Status

All twenty targets build (`fuzz_targets/session_fixtures.rs` holds
fixtures they share and is not one); Phase 4 added `vault_payload` and
`contact_store` and extended `credential_sequence`, Phase 5 added
`message_store` and `message_sequence`, and Phase 6 `ui_present` and
`ui_model_sequence`. In the final verification of Phase 6 (on `68ac655`)
each of the twenty ran for 26 seconds, and `ui_present` and
`ui_model_sequence` for 600 seconds each (1.2 and 2.4 million
executions), without a failure (`docs/DESIGN_QUESTIONS.md` 13.3). In
that of Phase 5 (on `91bf22c`) each of the eighteen ran for 25 seconds,
and `message_store`, `message_sequence`, `vault_payload` and
`contact_store` for 600 seconds each, without a failure
(`docs/DESIGN_QUESTIONS.md` 12.8). During Phase 4 the three ran for 60
seconds each from their seeds; `contact_store` found a confirmation for
a blocked identity that was reported as the wrong card instead of not
found, fixed in `1897a71`. In the final verification of Phase 4 (on
`970316f`) each of the sixteen ran for 25 seconds, and
`credential_sequence`, `contact_store`, `vault_payload`,
`session_frames`, the two handshake targets and the two Tor parsers for
600 seconds each, without a failure (`docs/DESIGN_QUESTIONS.md` 11.6).
In that of Phase 3 (on `2f27dcb`) the fourteen targets of then ran for
25 seconds each and six of them for 900 seconds, without a failure
(`docs/DESIGN_QUESTIONS.md` 10.4). Before that, the targets were run
from their seeds for short smoke tests only, last for all of them at the
end of Phase 2. The eight targets of the protocol core make between 77
thousand and 12 million executions in that time. The three session
targets make between 4 thousand and 16 thousand, because every execution
runs a handshake under the address sanitizer. After they gained time and
records, `session_frames` and `handshake_responder` also ran for 150
seconds each (25 thousand and 94 thousand executions), without a
failure. That shows the targets work. It is not a fuzzing campaign.

Phase 6 added `ui_present` and `ui_model_sequence`. Each ran for 60
seconds from its seeds on Linux under the address sanitizer (about 105
thousand executions for `ui_present`, 225 thousand for `ui_model_sequence`
in the last run, after the fixes below). The first runs of `ui_present`
found three faults in `monolith-ui-model`, all fixed with a regression
test in `present.rs`: a preview cut at a space had no ellipsis, a preview
with only white space after its last word had one, and a text whose lines
end in carriage return and line feed was never collapsed, because the two
form one grapheme cluster that was not counted as a line.
`ui_model_sequence` found nothing in its runs. Before the final
verification of Phase 6 the other targets were not run again for it. That
shows the two targets work. It is not a fuzzing campaign.

Not done yet:

- Long runs with a kept corpus, and a scheduled job.
- Targets for code that does not exist yet: the vault header.

## Rules for targets

- A target calls the decoder the application uses.
- Stateful targets compare what a peer can observe across the cases that
  must look alike.
- A check that restates a rule is written independently of the code it
  checks.
- Every crash becomes a regression test in the crate that owns the code.
