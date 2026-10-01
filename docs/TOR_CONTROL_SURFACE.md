# Tor control surface

This is the complete list of Tor control protocol commands Monolith can send.
The control client represents commands as a closed set of typed values; it
has no way to send anything that is not listed here. The onion-grater
profiles in `integrations/` allow the same set and nothing more.

Grammar and behavior are taken from the Tor control specification and the
tor 0.4.9.13 source. Sources are listed at the end.

## 1. Commands

### PROTOCOLINFO

    PROTOCOLINFO 1

- Why: learn which authentication methods the endpoint offers and the Tor
  version. Monolith refuses to work with a Tor older than the supported
  minimum.
- Platforms: all. On Whonix and Tails the reply comes from onion-grater,
  which reports `METHODS=NULL` and the real Tor version.
- Anonymity consequence: none. The version is used locally and never sent to
  a peer.

### AUTHENTICATE

    AUTHENTICATE
    AUTHENTICATE <hex>

- Why: required before any other command, even when no method is enabled.
- Platforms:
  - Whonix, Tails: bare `AUTHENTICATE`. The filter answers `250 OK` itself.
  - Other Linux: SAFECOOKIE (see AUTHCHALLENGE) when offered. COOKIE is used
    only if SAFECOOKIE is not offered; the specification marks it deprecated.
    HASHEDPASSWORD is used only if the user configured a control password.
- Anonymity consequence: none.
- Cookie handling: the cookie path comes from PROTOCOLINFO. The file must be
  exactly 32 bytes; anything else is rejected. The cookie is held in a
  redacted type and is not logged.

### AUTHCHALLENGE

    AUTHCHALLENGE SAFECOOKIE <client nonce hex>

- Why: SAFECOOKIE authentication. The client verifies the server hash before
  it answers, so it never reveals the cookie to something that is not Tor.
- Platforms: Linux without a control port filter. Not sent on Whonix or
  Tails: onion-grater filters it, and authentication there is `NULL`.
- Anonymity consequence: none.

### GETINFO status/circuit-established

    GETINFO status/circuit-established

- Why: the one status question Monolith asks. The reply is `0` or `1`.
- Platforms: all. It is in the default Whonix onion-grater allowlist.
- Anonymity consequence: none. Monolith deliberately does not read
  `status/bootstrap-phase`: its warning text can contain host addresses of
  guards or bridges, and filters replace it with a constant anyway.

### SETEVENTS HS_DESC

    SETEVENTS HS_DESC

- Why: Tor reports a successful descriptor upload only as an event. Monolith
  uses `HS_DESC UPLOADED` for its own service to move from "publishing" to
  "available". There is no GETINFO key for this.
- Platforms: Linux without a control port filter. Not sent on Tails or
  Whonix. Tor delivers `HS_DESC` events for every Onion Service of the
  instance, so a filter rule that lets them through would show one
  application, or on Whonix one Workstation, the addresses of the others.
  On those platforms the service is shown as published without upload
  confirmation (open item C3).
- Anonymity consequence: the raw event names the HSDir that received the
  descriptor and may concern services other than Monolith's. Monolith reads
  only the action and the service address, ignores events for other
  addresses, and discards the rest of the line.

### ADD_ONION

Exactly two shapes. `<target>` is defined per platform in section 2.

    ADD_ONION NEW:ED25519-V3 Flags=MaxStreamsCloseCircuit MaxStreams=8 PoWDefensesEnabled=1 Port=29170,<target>
    ADD_ONION ED25519-V3:<key> Flags=MaxStreamsCloseCircuit MaxStreams=8 PoWDefensesEnabled=1 Port=29170,<target>

`<key>` is the 88-character base64 blob Tor returned when the service was
first created. Both shapes always carry `PoWDefensesEnabled=1`; the minimum
supported Tor knows the keyword (`TOR_INTEGRATION.md` sections 1 and 6).

- Why: publish the Onion Service that peers connect to.
- The first shape creates a new service key. Tor returns it once in the
  reply (`250-PrivateKey=ED25519-V3:<key>`). Monolith stores it in the vault
  for a persistent endpoint, or holds it in memory for an ephemeral one so
  that the same service can be re-created if the control connection drops.
- The second shape re-creates a service from a stored key.
- Flags never sent: `Detach` (the service must die with Monolith),
  `NonAnonymous` (single onion mode is not anonymous), `DiscardPK` (the key
  is needed to survive a control connection loss), `V3Auth`.
- Platforms: all.
- Anonymity consequence: creates an Onion Service on this Tor instance. The
  service is anonymous in the ordinary Onion Service sense. If Tor is
  configured for non-anonymous single onion mode it answers
  `512 Tor is in non-anonymous hidden service mode`; Monolith treats that as
  fatal and refuses to publish.

### DEL_ONION

    DEL_ONION <service id>

- Why: remove the service on shutdown or when an endpoint is retired. Tor
  only lets a control connection remove services it created.
- Platforms: all.
- Anonymity consequence: none.

## 2. Target per platform

| Platform | `<target>` sent | What Tor connects to |
| --- | --- | --- |
| Linux, Tor on the same host | `127.0.0.1:<port>`, port chosen by the OS at bind time | Monolith's loopback listener |
| Tails | `29170` | `127.0.0.1:29170`, Monolith's loopback listener. The profile fixes the port, so it cannot be chosen at bind time. |
| Whonix | `29170` | onion-grater rewrites it to `<workstation address>:29170` |

On Tails and Whonix the complete argument string is fixed by the profile
regex. Monolith emits the keywords in exactly the order shown above.

## 3. Replies and events Monolith reads

| Line | Use |
| --- | --- |
| `250-AUTH METHODS=...` | choose authentication |
| `250-VERSION Tor="..."` | check the minimum supported version |
| `250 AUTHCHALLENGE SERVERHASH=... SERVERNONCE=...` | SAFECOOKIE |
| `250-status/circuit-established=0|1` | Tor status |
| `250-ServiceID=<56 base32>` | address of the published service |
| `250-PrivateKey=ED25519-V3:<88 base64>` | key of a newly created service |
| `650 HS_DESC UPLOADED <56 base32> ...` | service became available (Linux without a filter only) |
| `250 OK`, `5xx ...` | success and failure |

Every other line is ignored. Lines longer than `MAX_CONTROL_LINE_LEN` and
replies longer than `MAX_CONTROL_REPLY_LINES` end the control connection.
`510 Command filtered` is an ordinary error, not a reason to crash. The
reply parser is a fuzz target.

Arguments Monolith places in commands are generated locally and validated
before use: the service id must match `[a-z2-7]{56}`, the key must match
`[A-Za-z0-9+/]{86}==`, ports are integers. Peer input never reaches a
control command, so a peer cannot inject a line.

## 4. Commands Monolith never sends

`SETCONF`, `RESETCONF`, `GETCONF`, `LOADCONF`, `SAVECONF`, `SIGNAL` (including
`NEWNYM`), `MAPADDRESS`, `EXTENDCIRCUIT`, `SETCIRCUITPURPOSE`,
`ATTACHSTREAM`, `REDIRECTSTREAM`, `CLOSESTREAM`, `CLOSECIRCUIT`,
`POSTDESCRIPTOR`, `USEFEATURE`, `RESOLVE`, `TAKEOWNERSHIP`, `DROPGUARDS`,
`DROPOWNERSHIP`, `DROPTIMEOUTS`, `HSFETCH`, `HSPOST`,
`ONION_CLIENT_AUTH_ADD`, `ONION_CLIENT_AUTH_REMOVE`,
`ONION_CLIENT_AUTH_VIEW`, and every `GETINFO` key other than
`status/circuit-established`.

In particular Monolith never reads `GETINFO address`, guard or circuit
status, stream status, or configuration, and never changes guards, bridges,
circuits or any Tor option.

## 5. What enforces the list

- In the process: the typed command set. A test with a recording mock
  control port asserts that a full run emits only the lines above (T-CTRL-1).
- Outside the process, on Whonix and Tails: the onion-grater profile. A
  compromised Monolith on a Whonix-Workstation can still only issue what the
  merged Gateway profile allows. The profiles do not limit how many
  services a client creates; each one can only point at port 29170 of the
  client itself.
- Outside the process, on other Linux systems: nothing. Access to the
  control socket there is full control of Tor. This is a property of the
  platform, stated in `THREAT_MODEL.md`.

## 6. Open items

C1. `MaxStreams=8` with `MaxStreamsCloseCircuit`. One peer needs one stream
    per rendezvous circuit, so a small cap makes an attacker pay for a new
    circuit (and proof of work, when active) every few streams. Whether Tor
    counts simultaneous or cumulative streams per circuit has to be
    confirmed by test against tor 0.4.9 before Phase 3 fixes the value.

C2. `PoWQueueRate` and `PoWQueueBurst` are left at Tor's defaults (250 and
    2500), which are sized for busy services. Lower values may suit a
    personal endpoint better. Not changed without measurements.

C3. Confirming on Tails and Whonix that the service is reachable, without
    `HS_DESC` events. One option is for Monolith to dial its own Onion
    Service once after publishing and watch for the stream to arrive at
    its listener. That costs a circuit and reveals nothing new. Not
    specified yet.

## Sources

Accessed 2026-10-01.

- Tor control specification, commands and replies:
  https://spec.torproject.org/control-spec/commands.html
  https://spec.torproject.org/control-spec/replies.html
  https://spec.torproject.org/control-spec/implementation-notes.html
- tor 0.4.9.13 source, `src/feature/control/control_cmd.c`,
  `control_events.c`, `control.c`:
  https://gitlab.torproject.org/tpo/core/tor/-/tree/tor-0.4.9.13
- onion-grater source and profiles:
  https://github.com/Whonix/onion-grater
- Whonix default onion-grater profile:
  https://github.com/Whonix/anon-gw-anonymizer-config/blob/master/etc/onion-grater-merger.d/30_whonix-default.yml
