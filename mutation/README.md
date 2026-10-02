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

- `faults.py`: the lists `PHASE1`, `PHASE2` and `PHASE3`. An entry names the
  file, the exact text to replace and its replacement, and what the fault
  means. `expect` is `"caught"` or the reason why the fault survives.
- `run.py`: the runner.

## Running

    python3 mutation/run.py phase3 3
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

With the credential binding review and the integration hardening of
Phase 3:

| List | Faults | Caught | Expected to survive |
| --- | --- | --- | --- |
| Phase 1 | 45 | 43 | B3, B4 |
| Phase 2 | 99 | 95 | S15, S24, CAP2, CAP3 |
| Phase 3 | 72 | 69 | Q14, Q18, Q29 |

The Phase 3 list covers the no-clearnet and Tor boundaries (onion address
checks, SOCKS only, the onion name only, loopback endpoints and listener,
isolation), control authentication and the parser bounds, the publication
rules (no `Detach`, non-anonymous mode, key checks, ownership by the
control connection, `DEL_ONION`), the budgets and deadlines of the core,
and the checks the two Phase 3 reviews added (Q27 to Q32). For a fault in
`monolith-identity` or `monolith-protocol` the tests of `monolith-tor` and
`monolith-core` run as well.

The credential binding review added CR1 to CR21, and K2 to K4 and H16 in
the Phase 2 list. They cover the local party that only its own identity
key can make (K2 to K4), outbound admission against the state of the
moment and not of the dial (H16), a newer key without continuity, an
announcement through another key, the promotion and the retired key, the
pending card the user confirms or imported, a second statement at the
successor's epoch (CR1 to CR9, CR20, CR21), the withdrawal of a session
and a link and its end (CR10, CR12 to CR17, CR19), message 3 to a key
the contact has left (CR18), and the duplicate rule with credentials
(CR11). None is expected to survive. A doctest catches K4, so the runner
names no failing test for it.

The integration hardening added CR22 to CR38 and moved CR12, CR16, CR18
and CR23 to the code they test now; Q26 follows the code. They cover:

- message 3 only to a peer that may learn the local identity, read from
  the session and not from the admission returned beside it, with the
  withdrawal looked at before and during its write (CR18, CR22, CR36);
- the identity predicate on the proven key and not on the stale-card
  standing, an older card of the active key, and a proven key older
  than the announced successor (CR23, CR24, CR35);
- the deadlines of a session: an absolute frame deadline, the idle limit
  that bytes of a frame do not move, `UNKNOWN_FIRST_MESSAGE_TIMEOUT`,
  `UNKNOWN_SESSION_TIMEOUT`, the age limit without a complete frame, and
  a Close the session decided (CR25 to CR28, CR31, CR32);
- the one end of a link, which gives back its slot, after a deadline, a
  read error or a message that ends the session, refuses later calls,
  ends a pending send on a withdrawal, and reports a failed admission
  (CR16, CR29, CR30, CR33, CR34, CR38), and the slot of a stranger read
  from the session (CR37).

None is expected to survive. A slot for an outbound session whose
standing fell has no code to break: such a session is not made, which
CR18 tests. A stranger answered without a slot is Q30. A fuzz target that
compares accepted handshake messages with its transcript is not a fault
for this runner, which runs the tests; the seeds of a third party and of
another ephemeral key make such a target fail on its own corpus.

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
- Q14, equivalent. Tor sends nothing unasked; bytes that follow a reply
  stay in the socket and make the next reply fail, so the connection
  fails either way, one command later.
- Q18, equivalent. 88 characters ending in `==` always decode to 64
  bytes, so the length check of a returned key cannot fire.
- Q29, unreachable. `Endpoint` is opaque and only its validating
  constructors make one; the second check in the connection code cannot be
  reached from a test.

A survivor that is not in this list is a gap in the tests, or a check that
does nothing. Either is fixed before a phase is called done; F9 was such a
gap at the end of Phase 2, and in Phase 3 a handle that ignored control
loss before `accept`, a handshake without its deadline, and the dial and
listener-error paths of the core were; all have tests since.

## Adding a fault

Add an entry to the list of the phase that introduces the check. Keep the
replacement as small as the fault: one comparison, one bound, one call. If
the fault cannot be caught, say why in `expect`, and add the reason to the
list above.
