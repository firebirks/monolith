# Fuzzing

No fuzz targets exist yet. There is nothing to fuzz before Phase 1 adds the
decoders. This directory will hold a `cargo fuzz` project; it is kept out of
the main workspace because it needs a nightly toolchain.

Planned targets and the property each one enforces are listed in
`docs/TEST_PLAN.md` section 5.

Once targets exist:

    cargo install cargo-fuzz
    cargo +nightly fuzz run <target> -- -malloc_limit_mb=64 -timeout=5

Rules for targets:

- A target calls the same sans-IO decoder the application uses. No test-only
  parsing path.
- The allocation limit stays far below the default so that an allocation
  sized by input shows up as a failure.
- Stateful targets compare the persistent state before and after input from
  an unauthenticated peer.
- Every crash is turned into a regression test in the crate that owns the
  code.
