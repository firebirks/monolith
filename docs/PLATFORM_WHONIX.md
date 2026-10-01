# Whonix

Whonix is a primary target. Monolith runs in Whonix-Workstation and uses the
Tor of Whonix-Gateway through the Gateway's control port filter. It never
runs a Tor of its own.

Status: design. Nothing here has been run on Whonix yet. Statements about
Whonix come from the sources in section 10, read on 2026-10-01, mostly from
the Whonix source repositories. Items that could only be inferred are marked
"unverified" and are part of the test matrix in `TEST_PLAN.md`.

## 1. Platform facts

- Whonix 18.2.1.9 (2026-07-17), based on Debian 13. tor 0.4.9.x; run
  `anon-info` on the Gateway for the exact version.
- Non-Qubes Whonix (VirtualBox, KVM): LXQt on Wayland (labwc), Xwayland
  installed. Qubes-Whonix 18: X11, Qubes R4.3 only.
- Whonix 18 separates the normal user session from system maintenance.
  `sudo` is available only after booting into the SYSMAINT session, on the
  Gateway and on the Workstation. Every installation step below that needs
  root has to be done there.

Addresses:

| | Non-Qubes | Qubes |
| --- | --- | --- |
| Gateway, internal | `10.152.152.10` | dynamic; `10.152.152.10` is also accepted, the Gateway firewall redirects it (from the firewall source, unverified at runtime) |
| Workstation | `10.152.152.11` (more Workstations: `.12`, `.13`, ...) | dynamic, must not be hard-coded |

## 2. Architecture

    Whonix-Workstation                      Whonix-Gateway
    ------------------                      --------------
    monolith
      SOCKS5  -> 127.0.0.1:9050  ------->   Tor SocksPort 9050
      control -> 10.152.152.10:9051 ---->   onion-grater :9051
                                              -> Tor ControlPort 127.0.0.1:9052
      listener <ws-address>:29170  <-----   Tor (Onion Service target)

There is one Tor, on the Gateway. Tor over Tor does not occur because
Monolith contains no Tor and no code to start one (S4). This matters on
Whonix in particular: the mechanism Whonix uses to stop stacked Tor
(`anon-ws-disable-stacked-tor`) replaces the system `tor` binary and
occupies the default ports, but it cannot stop an application that embeds
its own Tor implementation. Not embedding one is Monolith's responsibility.

## 3. Outbound

- SOCKS endpoint: `127.0.0.1:9050`. The Workstation forwards this port to
  the Gateway's SocksPort 9050.
- Stream isolation: per-contact SOCKS5 credentials in the `<torS0X>0` format
  (`TOR_INTEGRATION.md` section 3.2). This is what Whonix recommends for
  Tor-aware applications instead of a dedicated SocksPort. Connections to
  different Onion Services are isolated from each other by Tor regardless.
- Whonix sets `TOR_SOCKS_IPC_PATH` to a Unix socket that forwards to the
  Tor Browser SocksPort (9150). Monolith uses it when its configuration says
  so; the default stays 9050 so that Monolith does not share the browser's
  port.

## 4. Inbound

Tor is on another machine, so the Onion Service target cannot be loopback.

### 4.1 Control connection

Monolith connects by TCP to the Gateway's onion-grater at
`10.152.152.10:9051` (configurable). What it sees:

- `PROTOCOLINFO` is answered by the filter with `AUTH METHODS=NULL` and the
  real Tor version.
- A bare `AUTHENTICATE` is answered `250 OK` by the filter.
- `AUTHCHALLENGE` is filtered. Monolith does not use SAFECOOKIE on Whonix.
- Anything outside the merged profile gets `510 Command filtered`. Monolith
  reports which command was refused and points to this document; it does not
  crash.

onion-grater opens one upstream control connection per client connection.
Tor scopes non-detached services to the connection that created them, so
Monolith's service is removed when Monolith's connection closes, and
Monolith can only delete services it created.

### 4.2 Listener address

Monolith binds the listener to the local address of its control connection.
That is the address the Gateway sees as the source of the connection, and
therefore exactly the address onion-grater substitutes for
`{client-address}` in the `ADD_ONION` target. It works without hard-coding
an address, on Qubes as well as elsewhere, and it never binds a wildcard
address.

If the control connection is made through a local forwarder (so that its
local address is loopback), Monolith cannot learn the right address this
way. It then requires `listen_address` in its configuration and refuses to
publish without one.

The Whonix documentation suggests listening on `0.0.0.0`. Monolith does not,
because the Workstation firewall rule that opens the port is not restricted
by source address or interface, and binding one address is the tighter
choice.

### 4.3 Port

The listener port is fixed at 29170. Two things need a fixed number: the
onion-grater profile, which has to name the target port, and the Workstation
firewall, which has to open it. Whonix's guidance for application developers
asks for a specific port or a small range for the same reasons.

### 4.4 Who can reach the listener

The Gateway's Tor, and any other machine on the same internal network, which
means other Workstations attached to the same Gateway. A hostile Workstation
can connect to the listener directly without going through Tor. It gains
little by that. It has to pass the same handshake and identity proof as
any peer and is subject to the same budgets. It does learn one thing a
remote peer cannot: if it holds the user's contact card, it can confirm
that this identity runs on the machine at that internal address. This is
why application-layer authentication is mandatory even though "traffic
comes from the Gateway", and a reason to give Monolith a Gateway of its own
when other Workstations are not trusted.

## 5. onion-grater profile

`integrations/whonix/onion-grater/40_monolith.yml` is a draft. It allows:

- `ADD_ONION` in the two exact shapes of `TOR_CONTROL_SURFACE.md`, with the
  target rewritten to `{client-address}:29170`;
- `DEL_ONION` with a 56-character service id.

It adds no event. Tor delivers `HS_DESC` events for every Onion Service on
the Gateway, and passing them would show each Workstation the addresses of
services created from the others. Monolith therefore shows its service as
published without upload confirmation on Whonix.

`GETINFO status/circuit-established` is already in the Whonix default
profile, so the Monolith profile does not add any `GETINFO` rule.

Properties of onion-grater on Whonix that shape the profile:

- All enabled profiles, plus the default one, are merged into a single
  filter that applies to every Workstation behind that Gateway. A profile
  can only add permissions. There is no per-application profile on Whonix,
  and the `users` and `apparmor-profiles` qualifiers have no effect. Real
  separation between applications needs separate Workstations, ideally
  separate Gateways.
- There is no special handling of `ADD_ONION`. A rule without a replacement
  forwards the command unchanged, and a bare target port would then mean a
  port on the Gateway itself. Every Monolith rule therefore pins the target
  to `{client-address}`.
- Flags are restricted only by what the regular expressions match. The
  profile spells out the full argument string; `Detach`, `NonAnonymous` and
  anything else do not match.
- Patterns contain no `.*` or `.+`. A loose pattern in another
  application's profile was a deanonymization bug fixed in Whonix in June
  2026.

Consequence worth stating: enabling the Monolith profile on a Gateway lets
any Workstation behind it create Onion Services, in any number, each
pointing at that Workstation's own port 29170. That is all it adds to the
Gateway's filter.

The profile carries the same three qualifier keys as the profiles Whonix
ships, so that the merger combines it with them.

## 6. Installation

Two sides, matching two packages when Monolith is packaged.

### 6.1 Gateway: `monolith-whonix-gateway-integration`

Ships the profile as
`/usr/share/doc/onion-grater-merger/examples/40_monolith.yml`. The user
enables it:

    sudo onion-grater-add 40_monolith

This links the profile into `/usr/local/etc/onion-grater-merger.d/` and
restarts onion-grater. `sudo onion-grater-remove 40_monolith` undoes it.

By hand, without the package (SYSMAINT session on the Gateway):

    sudo install -m 0644 40_monolith.yml /usr/local/etc/onion-grater-merger.d/40_monolith.yml
    sudo systemctl restart onion-grater

On Qubes-Whonix the file has to be a real file under
`/usr/local/etc/onion-grater-merger.d/` in the Gateway qube, or installed in
the Gateway template; a link into the template's example directory does not
exist in an app qube unless the package is installed in the template.

The long-term home of the profile is the Whonix onion-grater package itself,
which already ships opt-in profiles for other applications and accepts them
from upstream projects.

### 6.2 Workstation: `monolith`

Ships the binary and a firewall drop-in
`/etc/whonix_firewall.d/40_monolith.conf`:

    EXTERNAL_OPEN_PORTS+=" 29170 "

Reload with `sudo whonix_firewall` (SYSMAINT session) or reboot the
Workstation.

By hand, the same line goes into
`/usr/local/etc/whonix_firewall.d/50_user.conf`. On Qubes-Whonix that
location is in the app qube and persists; `/etc/whonix_firewall.d/` belongs
to the template.

No Gateway firewall change is needed. Monolith does not ask for, and must
not be given, any other Gateway access.

### 6.3 What Monolith refuses to do

Installation shortcuts that would weaken the Whonix design are out of scope:
no direct access to the Gateway's real control port (9052), no SSH or shared
folder to push configuration to the Gateway, no instruction to disable
onion-grater or to run it in complain mode outside of development, and no
bundled Tor.

## 7. Storage

Whonix-Workstation is an ordinary persistent system. Identity and contacts
are kept in the encrypted vault by default; message history is off by
default (`STORAGE.md`). In a disposable Workstation everything is gone with
the VM, and Monolith shows the session as ephemeral only if it was started
in ephemeral mode; it cannot detect a disposable VM reliably and does not
try.

## 8. Platform detection

Marker files: `/usr/share/anon-ws-base-files/workstation` (Workstation) and
`/usr/share/whonix/marker` (Whonix in general). Detection selects defaults:
control endpoint on the Gateway, fixed listener port, no SAFECOOKIE. It is
never sent to a peer.

Monolith never launches Tor, so `TOR_SKIP_LAUNCH=1` needs no handling.

## 9. Open items

W1. Run the draft profile on a Gateway. In particular confirm the
    replacement syntax, that the `PrivateKey` reply line reaches the
    Workstation for the `NEW` shape, and that the profile merges with the
    default one as intended.

W2. Confirm that a direct TCP connection to `10.152.152.10:9051` works from
    a Qubes-Whonix Workstation and that its local address is the one
    onion-grater substitutes.

W3. Confirm that the firewall drop-in in `/etc/whonix_firewall.d/` is read
    and that the port opens in the default firewall mode.

W4. Offer the profile to the Whonix project once the command shapes are
    frozen.

W5. Whether to use a per-application custom SocksPort on the Gateway instead
    of 9050. Current answer: no, follow the Whonix guidance for Tor-aware
    applications.

## 10. Sources

Accessed 2026-10-01.

- Release: https://forums.whonix.org/t/whonix-18-2-1-9-point-release/23402
- onion-grater source, merger and example profiles:
  https://github.com/Whonix/onion-grater
- Developer documentation for onion-grater:
  https://www.whonix.org/wiki/Dev/onion-grater
- Default profile:
  https://github.com/Whonix/anon-gw-anonymizer-config/blob/master/etc/onion-grater-merger.d/30_whonix-default.yml
- Gateway Tor configuration:
  https://github.com/Whonix/anon-gw-anonymizer-config
- Workstation port forwarding and stacked Tor prevention:
  https://github.com/Whonix/anon-ws-disable-stacked-tor
  https://www.whonix.org/wiki/Dev/anon-ws-disable-stacked-tor
- Firewall: https://github.com/Whonix/whonix-firewall
  https://www.whonix.org/wiki/Whonix-Workstation_Firewall
- Onion Services on Whonix: https://www.whonix.org/wiki/Onion_Services
- Stream isolation: https://www.whonix.org/wiki/Stream_Isolation
- Guidance for application developers:
  https://www.whonix.org/wiki/Dev/Project_friendly_applications_best_practices
- Tor over Tor:
  https://www.whonix.org/wiki/Tips_on_Remaining_Anonymous#Prevent_Tor_over_Tor_scenarios
- Network configuration:
  https://github.com/Whonix/whonix-gw-network-conf
  https://github.com/Whonix/whonix-ws-network-conf
