# ADR 0004: Tor backend

Status: proposed, pending Phase 0 review
Date: 2026-10-01

## Context

Monolith needs outbound connections to Onion Services and an Onion Service
of its own. On Tails and Whonix, Tor is provided by the platform and a
second Tor must never run. Elsewhere a system Tor is usually available. Arti
is a Rust implementation that can be embedded.

Other messengers show what goes wrong: Briar and Ricochet Refresh bundle
and launch their own Tor, and Whonix lists both as unsupported or not
recommended because of Tor over Tor. Cwtch can use a system Tor through
onion-grater on both platforms and is the only one of the three documented
to work there.

## Decision

1. All Tor access goes through one trait, `TorBackend`, in `monolith-tor`.
   Its operations are: status, connect to an Onion Service, publish a
   service that yields inbound streams, unpublish. Keys are opaque, targets
   are service keys (never hostnames or IP addresses), and nothing in the
   interface refers to SOCKS or the control protocol.

2. The first and, for version 1, only production implementation is
   `SystemTorBackend`: a SOCKS5 client and a Tor control client for an
   existing C tor.

3. Monolith never starts, configures or embeds Tor. There is no code path
   that launches a process.

4. The control client is written for this project and implements only the
   commands in `TOR_CONTROL_SURFACE.md`, as a closed set of typed commands.
   Existing controller libraries expose the whole protocol; here, not being
   able to send `SETCONF` is the point.

5. The SOCKS5 client is also minimal: one method (username and password),
   one command (CONNECT), one address type (domain name). It is small, has
   bounded reads, and is a fuzz target. A general SOCKS crate
   would bring address types and fallbacks that must not be used.

6. Services are created with `ADD_ONION`, never through torrc, never
   detached, never non-anonymous. Tor generates service keys; Monolith
   stores the returned key as an opaque blob.

7. Stream isolation uses per-contact SOCKS credentials in the `<torS0X>0`
   format. The platform chooses the SocksPort.

8. Tor's proof-of-work defense is always requested; the minimum supported
   Tor knows the keyword. Monolith implements no proof of work of its own.

9. `MockTorBackend` provides in-memory streams for tests. CI does not use
   the public Tor network.

10. `ArtiBackend` is deferred. When built, it lives in its own crate behind
    a feature that Tails and Whonix builds do not enable.

## Consequences

- The core cannot leak around Tor, because it has no socket code (S1, S2).
- The onion-grater profiles can be exact, because the command shapes are
  fixed and few.
- Monolith depends on behavior of C tor that it does not control: the
  service lives exactly as long as the control connection, and upload
  status is available only as an event, which the filters on Tails and
  Whonix cannot pass safely. On those platforms Monolith does not know
  when its descriptor has been uploaded.
- On Linux without a control port filter the user running Monolith needs
  access to Tor's control socket, which is full control of Tor.
- Two small protocol clients to write and maintain. Both face a local,
  mostly trusted daemon, are bounded, and are fuzzed.

## Alternatives considered

- Bundle C tor. Rejected: Tor over Tor on Whonix, a second Tor on Tails, and
  the maintenance of shipping Tor.
- Arti as the only backend. Rejected for the primary targets: it cannot use
  the platform's Tor, it has no control port for the platform filters to
  mediate, and its proof-of-work support is experimental. Arti's own
  documentation expects C tor to remain supported for years.
- Use an existing Rust controller library. Rejected for least privilege and
  dependency size; see decision 4.
- Unix socket as the Onion Service target instead of loopback TCP. Better
  isolation from other local users, but the Tor user needs access to the
  socket path and the platform profiles are written for TCP ports. Possible
  later hardening on plain Linux.

## Open questions

- `MaxStreams` value and semantics (TOR_CONTROL_SURFACE.md C1).
- Proof-of-work queue parameters (C2).
- Behavior of `ADD_ONION` under `Sandbox 1` on Tails (PLATFORM_TAILS.md T1).

## Sources

Accessed 2026-10-01.

- https://spec.torproject.org/control-spec/commands.html
- https://spec.torproject.org/socks-extensions.html
- https://gitlab.torproject.org/tpo/core/arti/-/blob/main/CHANGELOG.md
- https://arti.torproject.org/FAQs
- https://www.whonix.org/wiki/Chat
- https://docs.cwtch.im/docs/platforms/tails
- https://www.whonix.org/wiki/Cwtch
- https://code.briarproject.org/briar/briar-desktop/-/issues/294
