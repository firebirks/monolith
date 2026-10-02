# ADR 0004: Tor backend

Status: accepted for the generic system Tor backend, which Phase 3
implements. The Tails checks in `PLATFORM_TAILS.md` section 3.4 gate Phase 6
and any claim of Tails support, not the generic backend
(`DESIGN_QUESTIONS.md` T3-1).
Date: 2026-10-01; revised 2026-10-02 after the Phase 3 design review
(`DESIGN_QUESTIONS.md` section 5).

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
   Its operations are: status, a new isolation group, connect to an Onion
   Service, publish a service that yields inbound streams; the published
   service is closed through its handle. Keys are opaque, targets are
   service keys (never hostnames or IP addresses), the port is the protocol
   constant, and nothing in the interface refers to SOCKS or the control
   protocol.

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
   detached, never non-anonymous, in exactly two forms that differ only in
   the key: a new key that Tor returns once, or a key the caller holds.
   `DiscardPK` is not used, so that an endpoint can be published again
   after a lost control connection (T3-2). Tor generates service keys;
   Monolith holds the returned key as an opaque secret.

7. Stream isolation uses SOCKS credentials in the structured `<torS0X>0`
   format, one random token per contact, made by the adapter and never
   exposed outside it (T3-4). With several local identities the scope is
   the local identity and the contact. The platform chooses the
   SocksPort.

8. Tor's proof-of-work defense is always requested, with Tor's default
   queue parameters; Monolith cannot verify that it is active (T3-5).
   `MaxStreams=8` with `MaxStreamsCloseCircuit` limits streams per
   rendezvous circuit, provisionally (T3-6). Monolith implements no proof
   of work of its own.

9. Control authentication is SAFECOOKIE, or no authentication when the
   configuration says the endpoint is a trusted filter. Monolith reads
   `status/circuit-established` and, reduced to a progress number,
   `status/bootstrap-phase` (T3-3). It subscribes to no event.

10. `MockTorBackend` provides in-memory streams for tests. CI does not use
    the public Tor network.

11. `ArtiBackend` is deferred. When built, it lives in its own crate behind
    a feature that Tails and Whonix builds do not enable.

## Consequences

- The core cannot leak around Tor, because it has no socket code (S1, S2).
- The onion-grater profiles can be exact, because the command shapes are
  fixed and few.
- Monolith depends on behavior of C tor that it does not control: the
  service lives exactly as long as the control connection, and upload
  status is available only as an event, which names every service of the
  Tor instance. Monolith does not subscribe to it on any platform and does
  not know when its descriptor has been uploaded.
- On Linux without a control port filter the user running Monolith needs
  access to Tor's control socket, which is full control of Tor.
- Two small protocol clients to write and maintain. Both face a local,
  mostly trusted daemon, are bounded, and are fuzzed.
- The backend holds no identity state. Each publication has its own
  control connection, listener and handle, and isolation groups are kept
  by the caller, so one backend can serve several local identities at
  once (`TOR_INTEGRATION.md` section 4.3). Where a platform profile fixes
  the target port, only one of them can receive streams; see the open
  questions.

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

- `MaxStreams` value, provisionally 8 (TOR_CONTROL_SURFACE.md C1), for
  Phase 4.
- Proof-of-work queue parameters (C2).
- Behavior of `ADD_ONION` under `Sandbox 1` on Tails. This blocks Tails
  support, Phase 6 (PLATFORM_TAILS.md section 3.4), not the generic
  backend.
- A target port per local identity in the Tails and Whonix profiles, so
  that several identities can receive streams there
  (`DESIGN_QUESTIONS.md` MI-1).

## Sources

Accessed 2026-10-01; the Phase 3 sources are in `DESIGN_QUESTIONS.md`
section 5.4.

- https://spec.torproject.org/control-spec/commands.html
- https://spec.torproject.org/socks-extensions.html
- https://gitlab.torproject.org/tpo/core/arti/-/blob/main/CHANGELOG.md
- https://arti.torproject.org/FAQs
- https://www.whonix.org/wiki/Chat
- https://docs.cwtch.im/docs/platforms/tails
- https://www.whonix.org/wiki/Cwtch
- https://code.briarproject.org/briar/briar-desktop/-/issues/294
