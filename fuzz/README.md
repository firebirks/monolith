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
| `frame_plaintext` | state selector, plaintext | fails, or yields a message type legal in that state and a body that re-encode to the input |
| `frame_stream` | state, piece size, byte stream | fails, or yields payloads of legal length; never holds more than one frame |
| `text_fields` | bytes | every validator accepts or rejects; accepted values hold their invariants; save names are single safe path components |
| `session_sequence` | progress, standing, message types | no application data before confirmation; no backward transition; identities that are not contacts look alike |

The property all of them share: for arbitrary input the code either rejects
it or produces a valid bounded value, without a panic, without an allocation
sized by the input, and without an unbounded loop.

## Running

    cargo install cargo-fuzz
    cd fuzz
    cargo +nightly fuzz run <target> -- -malloc_limit_mb=64 -timeout=5

Add `-max_total_time=<seconds>` for a bounded run. The allocation limit is
deliberately far below the default, so that an allocation sized by input
shows up as a failure.

On Windows the address sanitizer runtime that ships with the MSVC toolchain
(`clang_rt.asan_dynamic-x86_64.dll`) has to be on `PATH`.

## Status

All eight targets build and have been run for short smoke tests only
(about twenty seconds each, between 0.3 and 5 million executions, no
failure). That shows the targets work. It is not a fuzzing campaign.

Not done yet:

- Seed corpora. Without a valid signed card as a seed, `contact_card` and
  the card-carrying messages in `message_body` mostly exercise the paths
  that reject. Seeds generated from the test vectors come next.
- Long runs with a kept corpus, and a scheduled job.
- Targets for code that does not exist yet: the handshake records, the SOCKS
  and control reply parsers, the vault header.

## Rules for targets

- A target calls the decoder the application uses.
- Stateful targets compare what a peer can observe across the cases that
  must look alike.
- Every crash becomes a regression test in the crate that owns the code.
