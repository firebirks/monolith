# Tor integration

Monolith uses a Tor that already exists on the system. It never starts one,
never embeds one and never configures one. This document describes how it
talks to that Tor. Platform specifics are in `PLATFORM_TAILS.md` and
`PLATFORM_WHONIX.md`; the control commands are in `TOR_CONTROL_SURFACE.md`.

Status: provisional. Nothing here is implemented, and implementation does
not start before the Tails preconditions in `PLATFORM_TAILS.md` section 3.4
have been checked.

Facts about Tor below were checked against the sources listed in section 11
on 2026-10-01.

## 1. Supported Tor

- C tor, series 0.4.9, from 0.4.9.5 (the first stable release of the
  series). Monolith refuses to use an older Tor. The 0.4.8 series reached
  end of life on 2026-06-01. Debian 13 ships 0.4.9.11 with 0.4.9.13 in the
  security archive; Tails 7.14 ships 0.4.9.13; Whonix 18 ships 0.4.9.x.
- 0.4.9.13 or later is recommended. It fixes TROVE-2026-053, a stream
  isolation bug in which an onion service circuit could be reused by a
  stream from a different isolation context. `monolith doctor` warns on
  older versions.
- Onion Service v3 only. Version 2 cannot be expressed in any Monolith data
  structure.

## 2. Backend abstraction

Everything goes through the `TorBackend` trait in `monolith-tor`:

    status()                    is Tor usable
    connect_onion(key, port, isolation token)  -> stream
    publish_onion_service(request)             -> service (accepts streams)
    unpublish_onion_service(service)

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
Monolith uses the format Tor specifies for this purpose:

    username = "<torS0X>0"
    password = lower-case hex of the contact's isolation token

The isolation token is 16 random bytes generated per contact at process
start and kept in memory only. All connections to one contact share it; no
two contacts do. Tor 0.4.9.1 and later parse this format. Older versions
treat it as ordinary credentials, with the same isolating effect.

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

Monolith does not set any isolation flag itself and does not change the
SocksPort configuration (S5).

## 4. Inbound connections

    peer -> Tor -> local listener -> Monolith

1. Monolith opens a control connection and authenticates.
2. It binds a listener (address per platform, section 7; on Whonix the
   address is taken from the control connection).
3. It sends `ADD_ONION` with the listener as the target
   (`TOR_CONTROL_SURFACE.md`).
4. Tor publishes the service descriptor. On Linux without a control port
   filter Monolith waits for `HS_DESC UPLOADED` for its own address and
   then reports the service as available. On Tails and Whonix that event is
   not used, and the service is reported as published once `ADD_ONION`
   succeeded.
5. Streams arriving at the listener are handed to the core as inbound
   sessions.

The control connection stays open for as long as the service should exist.
Tor removes a service when the control connection that created it closes.
Monolith relies on this: a crash or a kill takes the service down without
any cleanup step, and nothing is left behind in Tor.

If the control connection is lost while Monolith is running, the service is
gone. Monolith reports "Tor disconnected", reconnects with backoff and
creates the service again from the key it holds. `Detach` is never used.

### 4.1 Service keys

| Endpoint | Key | After a control connection loss | After exit |
| --- | --- | --- | --- |
| Persistent | Created by Tor on first use, returned once, stored in the vault | Re-created from the vault | Same address at next start |
| Ephemeral | Created by Tor, returned once, held in memory only | Re-created from memory | Gone |

Tor generates the key in both cases. The format Tor uses for the secret key
(scalar and PRF secret, not a seed) is specific to its key blinding scheme.
Monolith treats the blob as opaque and hands it back unchanged, which avoids
implementing that derivation.

### 4.2 Listener

- Tor on the same machine: `127.0.0.1`, port chosen by the operating system,
  except where the platform profile fixes the port (Tails).
- Tor on another machine (Whonix): one specific internal address and a fixed
  port. See `PLATFORM_WHONIX.md` section 4.
- Never a wildcard address.

The listener is reachable by local processes without going through Tor. That
is acceptable because the listener gives nothing to a party that cannot pass
the handshake and identity proof, and because every budget in
`RESOURCE_LIMITS.md` applies to local connections as well. It is the reason
application-layer authentication is mandatory even when traffic can only
arrive from Tor.

## 5. Status

Monolith asks one question, `GETINFO status/circuit-established`, and maps
the answer to `NotReady` or `Ready`. It does not read bootstrap progress or
any circuit, guard, stream or address information.

User-visible states derived from this and from the service lifecycle:
Tor disconnected, Tor connecting, Onion Service publishing, Onion Service
available.

## 6. Denial-of-service defenses provided by Tor

Monolith uses the mechanisms Tor offers and implements none of its own at
this layer.

- Proof of work. Tor's Onion Service proof-of-work defense (proposal 327) is
  requested with `PoWDefensesEnabled=1` on `ADD_ONION`. The keyword exists
  from tor 0.4.9.2, below Monolith's minimum, so it is always sent.
  - The defense needs the `pow` module, which is compiled in only when tor
    is built with GPL code enabled. Debian's packages are. Tor does not
    report the module over the control port, and `ADD_ONION` succeeds
    without it; the defense is then silently inactive. Monolith therefore
    cannot confirm that proof of work is active and does not claim it is.
  - With the defense on and no attack, the required effort is zero and
    clients connect as usual.
- Streams per circuit. `MaxStreams` with `MaxStreamsCloseCircuit` caps the
  number of streams on one rendezvous circuit (open item C1 in
  `TOR_CONTROL_SURFACE.md`).
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
| Control auth | SAFECOOKIE | none (filter) | none (filter) |
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

Accessed 2026-10-01.

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
