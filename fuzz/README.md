# Fuzzing

A `cargo fuzz` project for the protocol core. It is not a member of the main
workspace: it needs a nightly toolchain and has its own lock file.

Every target calls the same sans-IO code the application uses; there is no
test-only parsing path.

## Targets

| Target | Input | Property |
| --- | --- | --- |
| `base32` | text | fails, or decodes to bytes that encode back to the same text |
| `contact_card` | bytes | fails, or decodes to a card that encodes to the same bytes and whose signature verifies |
| `contact_card_text` | text | fails, or parses to a card whose canonical text parses to the same card |
| `message_body` | type selector, body | fails, or decodes to a message that encodes to the same bytes |
| `frame_plaintext` | state selector, mode, then a plaintext or the parts of one | fails, or yields a message type legal in that state and a body that re-encode to the input; an undamaged frame is accepted exactly when its type is legal in the state |
| `frame_stream` | state, piece size, mode, then a byte stream or records to build one from | fails, or yields payloads of legal length; never holds more than one frame; undamaged frames all come out again |
| `text_fields` | bytes | every validator accepts exactly what the rules of PROTOCOL.md section 9 accept, written out a second time in the target; save names are single safe path components |
| `session_sequence` | progress, standing, events | no application data before confirmation; no backward transition; a card of another identity is always a violation; nothing is sent that `may_send` forbids; identities that are not contacts look alike |

The property all of them share: for arbitrary input the code either rejects
it or produces a valid bounded value, without a panic, without an allocation
sized by the input, and without an unbounded loop.

Two targets have a second input mode. Random bytes almost never form a
padded frame, so in that mode the target builds valid frames from the
input and then damages one byte if the input says so. The first lines of
each target file describe its input layout.

## Seeds

`seeds/<target>/` holds valid inputs for each target: signed contact cards
in both forms, one body and one frame of every message type, streams of
frames, text samples, and event sequences for the session logic. Without
them the fuzzer cannot get past a signature check or a padding check.

The seeds are generated, not written by hand. The test `fuzz_seeds` in
`monolith-protocol` builds them from fixed values and fails if a committed
seed is not what the current code produces. After a format change:

    MONOLITH_WRITE_FUZZ_SEEDS=1 cargo test -p monolith-protocol --test fuzz_seeds

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

## Status

All eight targets build and have been run from their seeds for short smoke
tests only (about 25 seconds each, between 30 thousand and 3 million
executions, no failure). That shows the targets work. It is not a fuzzing
campaign.

Not done yet:

- Long runs with a kept corpus, and a scheduled job.
- Targets for code that does not exist yet: the handshake records, the SOCKS
  and control reply parsers, the vault header.

## Rules for targets

- A target calls the decoder the application uses.
- Stateful targets compare what a peer can observe across the cases that
  must look alike.
- A check that restates a rule is written independently of the code it
  checks.
- Every crash becomes a regression test in the crate that owns the code.
