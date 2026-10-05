# Tor integration

Monolith uses a Tor that already exists on the system. It never starts one,
never embeds one and never configures one. This document describes how it
talks to that Tor. Platform specifics are in `PLATFORM_TAILS.md` and
`PLATFORM_WHONIX.md`; the control commands are in `TOR_CONTROL_SURFACE.md`.

Status: the generic system Tor backend is Phase 3 work. The Tails checks of
`PLATFORM_TAILS.md` section 3.4 gate Phase 6 and any claim that Monolith
works on Tails; they do not gate the generic backend (`DESIGN_QUESTIONS.md`
T3-1). Tails and Whonix are not supported yet.

Facts about Tor below were checked against the sources listed in section 11
on 2026-10-02.

## 1. Supported Tor

Two things are kept apart: the oldest Tor that has every feature Monolith
uses, and the Tor a user should run.

- Feature baseline: C tor 0.4.9.5. It is the first stable release with the
  proof-of-work keywords of `ADD_ONION` (added in 0.4.9.2-alpha) and with
  the `<torS0X>0` SOCKS isolation format (0.4.9.1-alpha). Monolith refuses
  a Tor whose `PROTOCOLINFO` reports an older version.
- Run a supported, security-updated Tor. As of 2026-10-02 only the 0.4.9
  series is supported upstream (0.4.8 reached end of life on 2026-06-01),
  and the latest release is 0.4.9.13. 0.4.9.13 or later is recommended: it
  fixes TROVE-2026-053, a stream isolation bug in which an onion service
  circuit could be reused by a stream from a different isolation context.
  `monolith doctor` says when the local Tor is older. A version at or above
  the baseline is not thereby safe; that depends on the patches it has.
- Tested with: none yet in this repository (no Tor in the development
  environment); see `TEST_PLAN.md`.
- The version comes from `PROTOCOLINFO` of the local Tor. Monolith never
  asks the network which Tor is current.
- Onion Service v3 only. Version 2 cannot be expressed in any Monolith data
  structure.

## 2. Backend abstraction

Everything goes through the `TorBackend` trait in `monolith-tor`:

    status()                              what state Tor is in
    isolation_group()                     a new isolation context
    connect_onion(key, isolation group)   -> stream to port 29170 of the key
    publish_onion(request)                -> service (accepts streams)
    service.close()                       DEL_ONION, then close

`monolith-core` and `monolith-protocol` see byte streams and nothing else.
They do not know which backend is in use and they contain no socket code.

Implementations:

- `SystemTorBackend` (Phase 3): SOCKS5 and the control protocol of an
  existing C tor, directly or through onion-grater.
- `MockTorBackend` (Phase 3): in-memory streams for deterministic tests. No
  network.
- `ArtiBackend`: not planned for v1, see section 10.

## 3. Outbound connections

    Monolith -> SOCKS5 -> Tor -> peer Onion Service

- SOCKS version 5 only. The request uses address type 0x03 (domain name)
  with the `.onion` name that the backend derives from the 32-byte service
  key. Tor does the name handling; nothing is resolved locally (S3).
- Monolith offers exactly one SOCKS authentication method, username and
  password (0x02). If the proxy does not select it, the connection attempt
  fails. This guarantees that isolation credentials are always in effect.
- Destination port is `ONION_VIRTUAL_PORT` (29170).
- The SOCKS endpoint is a loopback TCP address or a Unix socket. A
  non-loopback SOCKS address is accepted only on Whonix, and only if it is
  the Gateway address (see `PLATFORM_WHONIX.md`). This prevents a
  misconfiguration from sending SOCKS requests across a real network.
- If the SOCKS endpoint cannot be reached, or returns an error, the dial
  fails. There is no other transport to fall back to (S1).
- A dial is abandoned after `CONNECT_TIMEOUT` and can be cancelled at any
  point. Retries follow the schedule in `RESOURCE_LIMITS.md` section 7, with
  at most `MAX_CONCURRENT_DIALS` in flight.
- SOCKS replies are read with a hard bound (`MAX_SOCKS_REPLY_LEN`).

### 3.1 Errors

Tor's extended SOCKS5 error codes (0xF0 to 0xF7: descriptor not found,
introduction failed, rendezvous failed, and so on) are only returned when
the SocksPort has the `ExtendedErrors` flag. On a system Tor that Monolith
does not configure they are usually off. Monolith maps them to finer error
categories when they appear and otherwise works with the plain SOCKS5
codes. No behavior depends on them.

### 3.2 Stream isolation

Tor does not share a circuit between streams with different SOCKS5
credentials (`IsolateSOCKSAuth`, on by default for every SocksPort).
Monolith uses the structured format Tor specifies for this purpose, not
legacy username and password isolation:

    username = "<torS0X>0"
    password = lower-case hex of an isolation token

The policy (`DESIGN_QUESTIONS.md` T3-4) is one token per contact, and with
several local identities (`ARCHITECTURE.md` section 1.1) one token per
local identity and contact:

- The Tor adapter makes the token: 16 bytes from the operating system's
  CSPRNG. It contains nothing derived from an identity, an onion address,
  a fingerprint, a name or a contact number.
- The caller receives an opaque `IsolationGroup` that has no accessor for
  the bytes and passes it with every dial. The core of Phase 4 keeps one
  per contact, so that reconnects may reuse a rendezvous circuit and
  different contacts never share an isolation context; in Phase 3 the
  caller of `link::dial` supplies it, and `dev-chat` makes one per run.
- The token is runtime state. It is never stored, logged or shown, and a
  new one is made after a restart.
- The backend keeps no token. The core keeps one group per pair of local
  identity and remote identity: identity A dialing Bob, A dialing Claire
  and B dialing Bob use three independent tokens, even though A and B
  reach the same Bob. No token is derived from a key, an address, a name
  or a local label.

Tor 0.4.9.1 and later parse this format, and Monolith requires a later
Tor.

What this does and does not add:

- A rendezvous circuit is bound to one Onion Service, so connections to
  different contacts never share a circuit in the first place. This follows
  from the Tor source, not from a specification statement.
- The credentials additionally keep Monolith's streams apart from the
  streams of other applications that use the same SocksPort, which matters
  on Tails and Whonix where the port is shared.
- Monolith does not ask for, or depend on, `IsolateDestAddr`,
  `IsolateDestPort` or a dedicated SocksPort. The platform decides which
  port is used: `127.0.0.1:9050` on Tails and Whonix by default.
- Isolation does not prevent traffic correlation. An observer of both ends
  of a circuit, or of the guard and the timing of both parties, is not
  affected by it.

Monolith does not set any isolation flag itself and does not change the
SocksPort configuration (S5).

## 4. Inbound connections

    peer -> Tor -> local listener -> Monolith

1. Monolith opens a control connection and authenticates.
2. It binds a listener (address per platform, section 7; on Whonix the
   address is taken from the control connection).
3. It sends `ADD_ONION` with the listener as the target
   (`TOR_CONTROL_SURFACE.md`).
4. Tor publishes the service descriptor. Monolith subscribes to no event,
   because `HS_DESC` events name every Onion Service of the Tor instance,
   so it does not learn when the upload happened. The service is reported
   as published once `ADD_ONION` succeeded (open item C3 in
   `TOR_CONTROL_SURFACE.md`).
5. Streams arriving at the listener are handed to the core as inbound
   sessions.

The control connection stays open for as long as the service should exist.
Tor removes a service when the control connection that created it closes.
Monolith relies on this: a crash or a kill takes the service down without
any cleanup step, and nothing is left behind in Tor.

If the control connection is lost while Monolith is running, the service is
gone. The publication handle notices it the next time it is asked to accept
or to report its state, and from then on reports the service as not
published; it never claims that the service is still online. Publishing
again, from the key the caller holds, is the job of the supervisor in the
core, with the backoff of `RESOURCE_LIMITS.md` section 7, not of the
backend. `Detach` is never used.

The supervisor exists since Phase 4 (`monolith_core::supervisor`), one per
local identity. It publishes the service from the key the identity holds,
runs its accept loop, reports the service unavailable when the loop ends
because the service is gone, waits the next delay of the schedule, and
publishes it again under the same name; the backend checks the ServiceID
Tor returns against the expected key. A restart of Tor takes the same
path: the control connection closes, and the next attempt that finds Tor
again publishes the service. While Tor is unreachable, the attempts
slow down as the schedule says; there is never an attempt without a
delay, and nothing is tried outside Tor. An identity without a stored key
is not published again. The private network test restarts Tor and checks
that both services of a node come back under their names and that a
contact completes Noise XK with them again.

Shutdown: the handle's `close` sends `DEL_ONION` with a short deadline and
then closes the control connection. Dropping the handle without `close`
closes the connection, which removes the service as well.

### 4.1 Service keys

| Endpoint | Key | After a control connection loss | After exit |
| --- | --- | --- | --- |
| Persistent | Created by Tor on first use, returned once, stored in the vault before the identity is used (Phase 4) | Re-created from the vault | Same address at next start |
| Ephemeral | Created by Tor, returned once, held in memory only | Re-created from memory | Gone |

Tor generates the key in both cases. The format Tor uses for the secret key
(scalar and PRF secret, not a seed) is specific to its key blinding scheme.
Monolith treats the blob as opaque and hands it back unchanged, which avoids
implementing that derivation.

### 4.2 Listener

- Tor on the same machine: `127.0.0.1`, port chosen by the operating system,
  except where the platform profile fixes the port (Tails).
- Tor on another machine (Whonix): one specific internal address and a fixed
  port, and connections are accepted only from the Gateway address. See
  `PLATFORM_WHONIX.md` sections 4.2 and 4.5.
- Never a wildcard address.

The listener is reachable by local processes without going through Tor. That
is acceptable because the listener gives no protocol response to a party
that cannot produce a valid first message, and because every budget in
`RESOURCE_LIMITS.md` applies to local connections as well. It is the reason
application-layer authentication is mandatory even when traffic can only
arrive from Tor.

### 4.3 Several services

Several local identities each publish their own service at the same time.
The backend allows it as it is: every `publish_onion` opens its own
control connection, binds its own listener and returns its own handle, and
the backend keeps no state about the services it published. Ending one
publication does not touch another. An Onion Service key belongs to one
local identity, and Tor refuses to publish a key it already holds.

A stream reaches the listener of the service it was sent to, so the
handle that accepted it says which local identity it addresses. The
supervisor of each identity runs the accept loop of its service with that
identity: its party, its contact store and its budgets. Nothing about
this is visible to a peer.

Platforms whose filter profile fixes the target port (Tails, and Whonix,
section 7) give every service the same listener, and a stream would no
longer say which identity it addresses. Running several identities with
inbound streams there needs one port per identity in the profiles, for
example a small fixed range. Until the profiles allow that, those
platforms can publish one identity's service at a time. Trying a first
message against the key of every identity would also route a stream, at
the cost of one X25519 operation per identity for every stream, and is
not planned. Open item MI-1 in `DESIGN_QUESTIONS.md` section 7.

All services of a process are published by one Tor, which uses the same
guards for all of them and is up exactly when they are. That can link
them for an observer; `THREAT_MODEL.md` adversary S.

## 5. Status

A status query opens a control connection, authenticates, asks
`GETINFO status/circuit-established` and `GETINFO status/bootstrap-phase`,
closes the connection, and separately checks that the SOCKS endpoint
answers a SOCKS5 greeting. From that it reports one of:

- control unavailable: the control endpoint cannot be reached or does not
  speak the protocol;
- authentication failed;
- unsupported Tor: older than the feature baseline;
- not ready: Tor answers but has no circuit, with the bootstrap progress
  when Tor gave one;
- ready: Tor has a circuit;

and, independently, whether the SOCKS endpoint is reachable. Only the
progress number and whether the bootstrap is `done` are taken from the
bootstrap line (`TOR_CONTROL_SURFACE.md`). Monolith does not read any
circuit, guard, stream or address information, and its correctness does
not depend on bootstrap percentages or tags.

User-visible states derived from this and from the service lifecycle:
Tor disconnected, Tor connecting, Onion Service published, Onion Service
not published.

## 6. Denial-of-service defenses provided by Tor

Monolith uses the mechanisms Tor offers and implements none of its own at
this layer.

- Proof of work. Tor's Onion Service proof-of-work defense (proposal 327) is
  requested with `PoWDefensesEnabled=1` on `ADD_ONION`, with Tor's default
  queue parameters. The keyword exists from tor 0.4.9.2-alpha, below
  Monolith's baseline, so it is always sent (`DESIGN_QUESTIONS.md` T3-5).
  Requested does not mean verified active: Monolith does not run the tor
  binary or list its modules to find out.
  - The defense needs the `pow` module, which is compiled in only when tor
    is built with GPL code enabled. Debian's packages are. Tor does not
    report the module over the control port, and `ADD_ONION` succeeds
    without it; the defense is then silently inactive. Monolith therefore
    cannot confirm that proof of work is active and does not claim it is.
  - With the defense on and no attack, the required effort is zero and
    clients connect as usual.
- Streams per circuit. `MaxStreams=8` with `MaxStreamsCloseCircuit` caps
  the number of streams on one rendezvous circuit; it is not a limit for
  the service as a whole. Provisional (open item C1 in
  `TOR_CONTROL_SURFACE.md`, `DESIGN_QUESTIONS.md` T3-6).
- Introduction point rate limiting (`HiddenServiceEnableIntroDoSDefense`)
  cannot be set through `ADD_ONION` and is therefore not used.

These reduce the cost of a flood. They do not make a targeted flood against
a known Onion Service address harmless. Application budgets apply to every
stream that reaches Monolith regardless of what Tor did before.

## 7. Platform matrix

| | Linux, system Tor | Tails | Whonix-Workstation |
| --- | --- | --- | --- |
| SOCKS | `127.0.0.1:9050` (configurable) | `127.0.0.1:9050` | `127.0.0.1:9050` |
| Control | Unix socket `/run/tor/control` or `127.0.0.1:9051` | `127.0.0.1:951` (onion-grater) | Gateway `:9051` (onion-grater) |
| Control auth | SAFECOOKIE, cookie file `/run/tor/control.authcookie` (configurable) | none (filter) | none (filter) |
| Filter profile | none | `integrations/tails/` | `integrations/whonix/` |
| Listener | `127.0.0.1`, OS-chosen port | `127.0.0.1:29170` | Workstation internal address, port 29170 |
| Tor process | system service | Tails Tor | Gateway Tor |

On Linux without a filter, access to the control socket is full control of
Tor. Monolith still emits only the commands in `TOR_CONTROL_SURFACE.md`, but
nothing outside the process enforces it. Running Monolith under a user that
is a member of the `debian-tor` group is a decision the user makes knowingly;
`monolith doctor` states it.

Platform detection (Tails: `ID="tails"` in `/etc/os-release`; Whonix: marker
files) selects defaults and, on Whonix, permits the Gateway address as a
SOCKS and control endpoint that is not loopback. It disables no invariant,
and its result is never sent to a peer.

## 8. What Monolith never does

- Start, stop, reload or signal a Tor process.
- Read or write torrc, or send `SETCONF`.
- Select, drop or inspect guards, bridges, relays or circuits.
- Request single onion (non-anonymous) mode. If Tor is configured for it,
  `ADD_ONION` fails and Monolith refuses to publish.
- Create detached services.
- Ask Tor for the external address or for anything about other streams.
- Send `SIGNAL NEWNYM`.

## 9. Testing

- Unit and integration tests in CI use `MockTorBackend` only. CI never
  touches the public Tor network.
- The SOCKS client and the control client are tested against scripted local
  servers, including hostile ones (oversized replies, stalls, garbage).
- A private Tor network for end-to-end tests is built with Chutney, which is
  the Tor Project's tool for this and has onion service network templates.
  These tests are opt-in and local.
- Tests against the public network are manual. See `TEST_PLAN.md`.

## 10. Arti

Arti 2.6.0 (September 2026) is a full client and can host Onion Services. It
has no C tor compatible control port; services are configured through its
own API or RPC interface. Its proof-of-work support is experimental. The Tor
Project says a full replacement of C tor is years away.

Consequences:

- Arti is not a backend for Tails or Whonix. Those platforms provide C tor
  and forbid a second Tor.
- A future `ArtiBackend` would embed Arti for standalone installations on
  other systems. The trait is shaped so that it fits: services yield
  streams, keys are opaque, nothing in the interface mentions SOCKS or the
  control protocol.
- It is not built before `SystemTorBackend` is stable, and it is compiled
  only behind a feature that Tails and Whonix builds do not enable (S4).

## 11. Sources

Accessed 2026-10-01, and again on 2026-10-02 for the Phase 3 review
(`DESIGN_QUESTIONS.md` section 5.4).

- Control protocol: https://spec.torproject.org/control-spec/commands.html
- Onion address encoding: https://spec.torproject.org/rend-spec/encoding-onion-addresses.html
- Address validation: https://spec.torproject.org/rend-spec/deriving-keys.html
- SOCKS extensions: https://spec.torproject.org/socks-extensions.html
- SOCKS authentication format (proposal 351):
  https://spec.torproject.org/proposals/351-socks-auth-extensions.html
- Stream isolation: https://spec.torproject.org/path-spec/stream-isolation.html
- Proof of work: https://spec.torproject.org/hspow-spec/index.html
- tor manual and source at tag tor-0.4.9.13:
  https://gitlab.torproject.org/tpo/core/tor/-/tree/tor-0.4.9.13
- Release status:
  https://gitlab.torproject.org/tpo/core/team/-/wikis/NetworkTeam/CoreTorReleases
- Debian tor versions: https://qa.debian.org/madison.php?package=tor
- Arti: https://gitlab.torproject.org/tpo/core/arti/-/blob/main/CHANGELOG.md
  and https://arti.torproject.org/FAQs
- Chutney: https://gitlab.torproject.org/tpo/core/chutney
