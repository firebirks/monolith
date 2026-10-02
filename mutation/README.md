# Mutation testing

A list of single faults in the checks that matter, and a runner that
applies them one at a time and runs the tests. Every fault has to make a
test fail, unless its entry says why it cannot. A fault in authentication
logic that no test notices blocks a phase (`docs/TEST_PLAN.md` section
14).

This is not an automatic mutation tool. Each fault is chosen for a rule of
the specification: a comparison removed, a bound moved, a check skipped, a
key kept.

## Files

- `faults.py`: the two lists, `PHASE1` and `PHASE2`. An entry names the
  file, the exact text to replace and its replacement, and what the fault
  means. `expect` is `"caught"` or the reason why the fault survives.
- `run.py`: the runner.

## Running

    python3 mutation/run.py phase2 3
    python3 mutation/run.py phase1 3
    python3 mutation/run.py all 3 H7 S1 S2

The first argument selects the list, the second the number of workers,
and any further arguments restrict the run to those faults. The runner
needs `git`, `cargo` and Python 3, and nothing else.

It works on copies of the committed `HEAD` under `mutation/work/`, one per
worker, each with its own target directory, so the working tree is never
touched and uncommitted changes are not tested. Before anything runs it
checks that every pattern occurs exactly once in its file, and that the
unmodified copies pass their tests. For each fault it runs the tests of
the crate that holds the fault and of every crate that depends on it,
with `--no-fail-fast`, so the result names every test that noticed.

Outcomes:

- caught: a test failed, or the run hit the timeout;
- survived: every test passed;
- invalid: the fault does not compile, which is a mistake in the list.

The results go to `mutation/results-<list>-<commit>.json`. The exit code
is 0 if every fault had the outcome its entry expects, and 1 otherwise. A
full run of both lists takes about an hour with three workers.

When code moves, a pattern may no longer match. The runner then stops
before testing and names the entry; update the pattern so that the fault
stays the same fault.

## Expected results

At the end of Phase 2 (`1337009` and later):

| List | Faults | Caught | Expected to survive |
| --- | --- | --- | --- |
| Phase 1 | 45 | 43 | B3, B4 |
| Phase 2 | 98 | 94 | S15, S24, CAP2, CAP3 |

No fault in authentication logic survives. The Phase 2 list covers each
check that the two reviews of Phase 2 added: the grace after the age
limit (S1, S2), the refusal in `set_transfer_active` (S7), `accept_sent`
and `request_sent` in `may_send` (S13, S14), the invitation comparison
(S16), where the invitation comes from (H13, H14), each call of `end()`
that drops the keys (S20 to S25), the all-zero secret key (K1) and
`OutboundPeer::admit` with its record (H15, H16).

Why the six survive:

- B3, equivalent. The card inside a ContactRequest or EndpointUpdate is
  taken with the length of a card without capability, 171 bytes. A card
  whose flag announces a capability needs 187 and fails to decode before
  the check that refuses a capability is reached. The check stays as a
  second line.
- B4, equivalent. Without the early check of the endpoint count the same
  input is still refused, by `take()` or by `ContactCard::decode`.
  `take()` returns a slice and allocates nothing. Only the local error
  category can differ.
- S15, equivalent. A failed session has its logic in `Closed`, and one
  that made its Close is in `Closing`. The logic allows nothing there that
  `send()` could produce, so the two flags in `may_send` are a second line.
- S24, unreachable. The `Internal` path of `seal` is taken only if the
  Noise library refuses a length the frame encoder produces, which no test
  can arrange. `docs/CRYPTOGRAPHY.md` lists it.
- CAP2, not observable. The memory of a dropped value cannot be read
  without unsafe code, which the workspace forbids. The test checks that
  the type promises `ZeroizeOnDrop` and that `zeroize` clears it.
- CAP3, equivalent in result. A constant-time comparison is a timing
  property, which no test measures.

A survivor that is not in this list is a gap in the tests, or a check that
does nothing. Either is fixed before a phase is called done; F9 was such a
gap at the end of Phase 2 and has a test since.

## Adding a fault

Add an entry to the list of the phase that introduces the check. Keep the
replacement as small as the fault: one comparison, one bound, one call. If
the fault cannot be caught, say why in `expect`, and add the reason to the
list above.
