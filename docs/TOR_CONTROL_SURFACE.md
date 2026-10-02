# Tor control surface

This is the complete list of Tor control protocol commands Monolith can send.
The control client represents commands as a closed set of typed values; it
has no way to send anything that is not listed here, and no function that
takes a command, a GETINFO key or an option as a string. The onion-grater
profiles in `integrations/` allow a subset of the same set and nothing more.

Grammar and behavior are taken from the Tor control specification and the
tor source. The decisions behind this list are in `DESIGN_QUESTIONS.md`
section 5. Sources are listed at the end.

## 1. Commands

Every command is one line ending in CRLF. Every argument is produced by
Monolith from a typed, validated value; nothing a peer sends reaches a
command.

### PROTOCOLINFO

    PROTOCOLINFO 1

- Why: learn which authentication methods the endpoint offers, the cookie
  file for SAFECOOKIE, and the Tor version. Monolith refuses a Tor below
  the feature baseline (`TOR_INTEGRATION.md` section 1).
- Sent once per control connection, before authentication.
- Anonymity consequence: none. The version is used locally and never sent
  to a peer.

### AUTHCHALLENGE

    AUTHCHALLENGE SAFECOOKIE <client nonce, 64 hex digits>

- Why: SAFECOOKIE authentication. The client nonce is 32 random bytes.
  Monolith checks the server hash before it answers, so it never proves
  knowledge of the cookie to something that does not know the cookie
  itself.
- Sent only when the configuration says SAFECOOKIE and PROTOCOLINFO offers
  it.

### AUTHENTICATE

    AUTHENTICATE <64 hex digits>
    AUTHENTICATE

- The first form carries the SAFECOOKIE client hash.
- The second form is used only when the configuration says that the
  control endpoint is a trusted filter that answers authentication itself
  (onion-grater on Tails and Whonix, platform adapters of later phases).
  It is never chosen because a server offers `NULL`.
- COOKIE is not implemented: the specification marks it deprecated, and it
  sends the cookie itself. HASHEDPASSWORD is not implemented.
- A failed authentication ends the control connection.

### GETINFO status/circuit-established

    GETINFO status/circuit-established

- Why: whether Tor has a circuit. This decides "ready". The reply is `0`
  or `1`; anything else is a malformed reply.

### GETINFO status/bootstrap-phase

    GETINFO status/bootstrap-phase

- Why: to show bootstrap progress while Tor is not ready.
- The parser keeps two things: the number after `PROGRESS=` (0 to 100) and
  whether `TAG=` is `done`. Everything else on the line is dropped while
  parsing and never stored, logged or shown. That includes the warning
  form's `REASON`, `HOSTID` and `HOSTADDR`, which name a guard or bridge.
- Readiness does not depend on it. The specification guarantees only the
  tags `starting` and `done`; percentages and the order of phases may
  change, and Tor may revisit earlier phases.
- A refusal (for example `510` from a filter, or `552`) makes the progress
  unknown; it is not an error of the status query.
- The two keys are asked in two commands, because an unknown key makes
  Tor answer a combined GETINFO with an error and no values.

### ADD_ONION

Exactly two forms. They differ only in the key.

    ADD_ONION NEW:ED25519-V3 Flags=MaxStreamsCloseCircuit MaxStreams=8 PoWDefensesEnabled=1 Port=29170,<target>
    ADD_ONION ED25519-V3:<key> Flags=MaxStreamsCloseCircuit MaxStreams=8 PoWDefensesEnabled=1 Port=29170,<target>

- `<key>` is the 88-character base64 form of the 64-byte key that Tor
  returned when the service was first created: the ed25519 secret scalar
  and the PRF secret, in Tor's own format. It is not an Ed25519 seed and is
  not derived from any Monolith key.
- `<target>` is the address of the listener Monolith bound for this
  service. For a system Tor on the same host it is `127.0.0.1:<port>`, the
  port chosen by the operating system at bind time. Section 2 lists the
  platform forms.
- The first form creates a key; Tor returns it once in
  `250-PrivateKey=ED25519-V3:<key>`. The caller decides whether it is kept
  only in memory (ephemeral endpoint) or stored (persistent endpoint, from
  Phase 4). Phase 3 never writes it to disk.
- The second form publishes the service of a key the caller holds.
- Common policy, the same in both forms:
  - `MaxStreams=8` with `Flags=MaxStreamsCloseCircuit`. The limit applies
    per rendezvous circuit, not to the service as a whole: a circuit that
    tries to open more streams is closed. It is defense in depth; the
    application budgets of `RESOURCE_LIMITS.md` decide what is accepted.
    8 is provisional and reviewed in Phase 4.
  - `PoWDefensesEnabled=1`, with Tor's default queue parameters. Monolith
    requests the defense. It cannot tell whether the installed tor was
    built with the module that implements it, and does not claim that the
    defense is active.
- Never sent: `Detach` (the service must end with Monolith's control
  connection), `DiscardPK`, `NonAnonymous`, `V3Auth`, `BasicAuth`,
  `ClientAuth`, `ClientAuthV3`, `PoWQueueRate`, `PoWQueueBurst`, and any
  `RSA1024` key.
- If Tor is configured for non-anonymous single onion mode it answers
  `512 Tor is in non-anonymous hidden service mode`. Monolith treats that as
  a configuration error, does not retry with `NonAnonymous`, and publishes
  nothing.

### DEL_ONION

    DEL_ONION <service id>

- Why: remove the service on an orderly shutdown. Tor lets a control
  connection remove only services it created. If the command cannot be
  sent, closing the control connection removes the service anyway.

## 2. Target per platform

| Platform | `<target>` sent | What Tor connects to |
| --- | --- | --- |
| Linux, Tor on the same host | `127.0.0.1:<port>`, port chosen by the OS at bind time | Monolith's loopback listener |
| Tails (Phase 6) | `29170` | `127.0.0.1:29170`, Monolith's loopback listener. The profile fixes the port, so it cannot be chosen at bind time. |
| Whonix (Phase 7) | `29170` | onion-grater rewrites it to `<workstation address>:29170` |

Phase 3 implements the first row only. On Tails and Whonix the complete
argument string is fixed by the profile regex, and Monolith emits the
keywords in exactly the order shown above.

## 3. Replies Monolith reads

| Reply | Use |
| --- | --- |
| `250-PROTOCOLINFO 1` | start of the PROTOCOLINFO reply |
| `250-AUTH METHODS=... [COOKIEFILE="..."]` | authentication methods and the cookie file |
| `250-VERSION Tor="..."` | the feature baseline check |
| `250 AUTHCHALLENGE SERVERHASH=<64 hex> SERVERNONCE=<64 hex>` | SAFECOOKIE |
| `250-status/circuit-established=0` or `=1` | readiness |
| `250-status/bootstrap-phase=...` | progress number and `done`, nothing else |
| `250-ServiceID=<56 base32>` | address of the published service |
| `250-PrivateKey=ED25519-V3:<88 base64>` | key of a newly created service |
| `250 OK` | success |
| `5xx ...` | failure; the code is kept, the text is not shown |

Rules for the parser:

- A line ends in CRLF. A bare LF or CR, a line longer than
  `MAX_CONTROL_LINE_LEN`, a reply with more than `MAX_CONTROL_REPLY_LINES`
  lines, or more than `MAX_CONTROL_REPLY_LEN` bytes ends the control
  connection.
- A status code is three digits followed by `-`, `+` or a space. The
  data-block form (`+`) is refused: no command Monolith sends needs it.
- Asynchronous `650` replies are refused: Monolith subscribes to no event,
  so Tor sends none.
- In a PROTOCOLINFO reply, unknown lines are ignored, as the specification
  requires. In every other reply, a line that is not in the table above is
  a malformed reply.
- The cookie file name is a C-style quoted string; it is decoded to bytes
  with the escapes Tor produces and nothing else.
- Every ServiceID Tor returns is decoded as an onion address, with its
  version and checksum, into an `OnionServiceKey`. When the service was
  published from a known key, the returned ServiceID must name that key.
- A returned private key must be exactly 88 base64 characters that decode
  to 64 bytes, and it must appear exactly when the first form was sent.
- `510 Command filtered` and every other `5xx` are ordinary errors.

## 4. Commands Monolith never sends

`SETEVENTS` (no event at all), `SETCONF`, `RESETCONF`, `GETCONF`, `LOADCONF`,
`SAVECONF`, `SIGNAL` (including `NEWNYM`), `MAPADDRESS`, `EXTENDCIRCUIT`,
`SETCIRCUITPURPOSE`, `SETROUTERPURPOSE`, `ATTACHSTREAM`, `REDIRECTSTREAM`,
`CLOSESTREAM`, `CLOSECIRCUIT`, `POSTDESCRIPTOR`, `USEFEATURE`, `RESOLVE`,
`TAKEOWNERSHIP`, `DROPGUARDS`, `DROPOWNERSHIP`, `DROPTIMEOUTS`, `HSFETCH`,
`HSPOST`, `ONION_CLIENT_AUTH_ADD`, `ONION_CLIENT_AUTH_REMOVE`,
`ONION_CLIENT_AUTH_VIEW`, `QUIT`, and every `GETINFO` key other than the
two above. In particular Monolith never reads `version` (PROTOCOLINFO has
it), `address`, `onions/current`, `onions/detached`, `network-liveness`,
guard, circuit or stream status, or configuration.

Phase 0 listed `SETEVENTS HS_DESC` to confirm the descriptor upload on
Linux without a filter. It was dropped in Phase 3: `HS_DESC` events cover
every Onion Service of the Tor instance, including those of other
applications. A service is reported as published once `ADD_ONION`
succeeded; confirming that it is reachable is open item C3.

## 5. What enforces the list

- In the process: the typed command set, with no string-taking command
  function. Tests check the exact bytes of every command and that a full
  run against a recording control server sends nothing else (T-CTRL-1).
- Outside the process, on Whonix and Tails: the onion-grater profile. A
  compromised Monolith on a Whonix-Workstation can still only issue what the
  merged Gateway profile allows.
- Outside the process, on other Linux systems: nothing. Access to the
  control socket there is full control of Tor. This is a property of the
  platform, stated in `THREAT_MODEL.md`.

## 6. Open items

C1. `MaxStreams=8`. The specification does not say whether Tor counts
    streams open at the same time or every stream a rendezvous circuit has
    carried. One Monolith session uses one stream, so either reading leaves
    room for reconnects on one circuit, and a closed circuit only forces a
    new one. Provisional; reviewed with the resource tuning of Phase 4.

C2. `PoWQueueRate` and `PoWQueueBurst` are left at Tor's defaults (250 and
    2500), which are sized for busy services. Lower values may suit a
    personal endpoint better. Not changed without measurements.

C3. Confirming that the service is reachable without `HS_DESC` events. One
    option is for Monolith to dial its own Onion Service once after
    publishing and watch for the stream to arrive at its listener. That
    costs a circuit and reveals nothing new. Not specified yet.

## Sources

Accessed 2026-10-02.

- Tor control specification:
  https://spec.torproject.org/control-spec/commands.html
  https://spec.torproject.org/control-spec/message-format.html
  https://spec.torproject.org/control-spec/replies.html
  https://spec.torproject.org/control-spec/implementation-notes.html
- Safe cookie authentication:
  https://spec.torproject.org/proposals/193-safe-cookie-authentication.html
- tor source, `src/feature/control/control_cmd.c`, `control_auth.c`,
  `control_getinfo.c`, `src/lib/log/escape.c`, at commit c6c6170
  (2026-09-29): https://gitlab.torproject.org/tpo/core/tor
- onion-grater source and profiles:
  https://github.com/Whonix/onion-grater
