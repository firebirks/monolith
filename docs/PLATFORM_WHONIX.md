# Whonix

Whonix is a primary target. Monolith runs in Whonix-Workstation and uses the
Tor of Whonix-Gateway through the Gateway's control port filter. It never
runs a Tor of its own.

Status: provisional. Nothing here has been run on Whonix. The isolation of
the listener from other Workstations (section 4.5) is specified but not
tested, and is not claimed to work until it has been tested with one Gateway
and two Workstations. That test is a precondition for Phase 7.

Statements about Whonix come from the sources in section 10, read on
2026-10-01, mostly from the Whonix source repositories. Items that could
only be inferred are marked "unverified" and are part of the test matrix in
`TEST_PLAN.md`.

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

The Whonix documentation suggests listening on `0.0.0.0`. Monolith does
not. It binds the one IPv4 address described above and no IPv6 address.
onion-grater is IPv4 only, so Tor always connects to that address.

### 4.3 Port

The listener port is fixed at 29170. Two things need a fixed number: the
onion-grater profile, which has to name the target port, and the Workstation
firewall, which has to open it. Whonix's guidance for application developers
asks for a specific port or a small range for the same reasons.

### 4.4 The problem: other Workstations

Only the Gateway's Tor should be able to reach the listener. Without extra
measures that is not the case when several Workstations share a Gateway.

Non-Qubes Whonix (VirtualBox, KVM with a shared internal network):

- All Workstations of a Gateway are on one virtual layer 2 segment and
  share a subnet. Whonix documents that they can detect each other and
  that a compromised one can impersonate another Workstation or the
  Gateway.
- The supported way to open a port, `EXTERNAL_OPEN_PORTS`, generates a rule
  with no source address and no interface. Whonix documents the result as a
  feature: one Workstation can then connect to the other directly.
- So a second Workstation can reach TCP port 29170 of the first, bypassing
  Tor, as soon as the port is opened the supported way.

Qubes-Whonix:

- Qubes uses routed point-to-point links, not a shared segment, and traffic
  between qubes behind the same net qube is blocked by default. A second
  Workstation cannot reach the listener and cannot take another qube's
  address.

What a Workstation that reaches the listener could do: nothing a remote
peer cannot, with one exception. It has to pass the same handshake and
authentication as any peer and is subject to the same budgets. But if it
holds the user's contact card, a successful handshake tells it that this
identity runs on the machine at that internal address. That links an
identity to a virtual machine, which Tor would otherwise have hidden. How
much it learns without the card depends on the session layer that is
chosen: some candidates in ADR 0002 show the identity key to any party that
connects.

### 4.5 Restricting the listener to the Gateway

Design, in layers. None of it has been tested.

1. Bind address. The listener is bound to the Workstation's internal IPv4
   address only (4.2). Never `0.0.0.0`, never an IPv6 address.

2. Source check in Monolith. The Gateway address is the remote address of
   Monolith's own control connection. A connection accepted on the listener
   whose source is any other address is closed at once, before a byte is
   read or written, so it does not even learn that the port speaks
   Monolith. This needs no privileges and no configuration, and it holds
   through every firewall reload. It is always on when the platform is
   Whonix.

3. Firewall rule restricted to the Gateway. The port is not put into
   `EXTERNAL_OPEN_PORTS`. Instead one rule is inserted into the Workstation
   firewall's input chain that accepts new connections to port 29170 from
   the Gateway address only:

       nft insert rule inet filter input ip saddr <gateway> tcp dport 29170 ct state new counter accept

   `<gateway>` is `10.152.152.10`, or on Qubes the value of
   `qubesdb-read /qubes-gateway`, the same way the Whonix firewall itself
   determines it. The chain's policy is drop, so without the rule the port
   is closed: this fails closed.

   Whonix has no supported variable or hook for a rule like this. The
   firewall script has no per-source option, and configuration files in
   `/etc/whonix_firewall.d/` are read before the tables exist, so a rule
   cannot be added from there. The one mechanism the Whonix wiki describes
   is a systemd drop-in for `whonix-firewall.service` with an
   `ExecStartPost` command, and it labels that "Testers only! Unsupported!".
   The files in `integrations/whonix/firewall/` use it. Known limit: a
   manual `sudo whonix_firewall` reload rebuilds the table without running
   the unit, so the rule is gone until the service is restarted. The port
   is closed in the meantime, which is the safe direction.

4. Control port policy. onion-grater cannot limit the Monolith profile to
   one Workstation. Its merger unions the `hosts` lists, the default
   profile has `hosts: '*'`, and Whonix calls per-host rules unsupported.
   The profile governs who may create a service, not who may reach the
   listener, so it does not bear on this problem either way.

5. Topology. Layers 2 and 3 filter by source address. On a shared layer 2
   segment a compromised Workstation with root can take the Gateway's
   address or answer its ARP requests, and is then past both. Whonix says
   the same about its own source-based measures. Only separation below the
   IP layer closes this:

   - Qubes-Whonix, where the problem does not arise;
   - one Gateway per Workstation, which is what Whonix recommends for
     applications that use onion-grater;
   - a separate internal network per Workstation (Whonix's instructions for
     this are marked as an unfinished draft);
   - on KVM, isolated ports on the Workstation interfaces.

   Monolith cannot set any of these up. Its documentation tells the user
   that running Monolith next to untrusted Workstations on one non-Qubes
   Gateway is not a supported configuration, and why.

What remains after all layers, on a shared non-Qubes segment: a root-level
attacker in another Workstation who impersonates the Gateway reaches the
handshake. Authentication is unchanged by any of the above: such an
attacker is an unknown peer, cannot impersonate a contact, and learns what
section 4.4 describes. Layers 1 to 3 remove the case of an unprivileged or
merely curious process in another Workstation; they do not remove this one.

Not used: pinning the Gateway's hardware address with a static neighbor
entry. Whonix mentions the idea as "quickly researched" only, hardware
addresses can be forged as well, and the effect depends on the virtual
switch.

A supported per-source way to open a port would be a better layer 3 than an
unsupported drop-in. Asking the Whonix project for one is open item W6.

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

Ships the binary and the firewall integration of section 4.5, layer 3:

| File | Purpose |
| --- | --- |
| `/usr/libexec/monolith/monolith-listener-rule` | inserts the rule that accepts port 29170 from the Gateway only |
| `/usr/lib/systemd/system/whonix-firewall.service.d/40_monolith.conf` | runs it after the Whonix firewall has loaded |

Apply with `sudo systemctl restart whonix-firewall` (SYSMAINT session) or
reboot the Workstation. After a manual `sudo whonix_firewall` reload the
rule is gone and the service has to be restarted.

The port must not also be listed in `EXTERNAL_OPEN_PORTS`. That would open
it to every source and undo the restriction.

The supported alternative is the single line

    EXTERNAL_OPEN_PORTS+=" 29170 "

in `/usr/local/etc/whonix_firewall.d/50_user.conf`. It is simpler and
survives reloads, and on non-Qubes Whonix it leaves the port open to every
Workstation on the Gateway, with only Monolith's own source check in front
of the handshake. It is acceptable on Qubes-Whonix, and on a Gateway that
serves a single Workstation.

On Qubes-Whonix, files under `/usr/local` are in the app qube and persist;
files under `/etc` and `/usr/lib` belong to the template.

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

W3. Test section 4.5 with one Gateway and two Workstations, on VirtualBox
    or KVM and on Qubes-Whonix: that the rule is inserted and survives a
    service restart; that the second Workstation cannot connect; that
    Monolith's source check closes a connection that gets past the
    firewall; what a Workstation that takes the Gateway's address achieves.
    Precondition for Phase 7. Until then the isolation is a plan.

W4. Offer the profile to the Whonix project once the command shapes are
    frozen.

W5. Whether to use a per-application custom SocksPort on the Gateway instead
    of 9050. Current answer: no, follow the Whonix guidance for Tor-aware
    applications.

W6. Ask the Whonix project for a supported way to open a Workstation port
    for the Gateway only, so that the unsupported systemd drop-in is not
    needed.

W7. Which KVM network design the released Whonix 18 images use: the shared
    internal bridge or the newer point-to-point link. It decides whether
    the shared-segment problem exists there at all.

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
- Workstation firewall script and its configuration handling:
  https://github.com/Whonix/whonix-firewall/blob/master/usr/bin/whonix-workstation-firewall
  https://github.com/Whonix/whonix-firewall/blob/master/usr/libexec/whonix-firewall/firewall-common
- Custom firewall rules through a systemd drop-in:
  https://www.whonix.org/wiki/Whonix-Workstation_Firewall
- Several Workstations on one Gateway:
  https://www.whonix.org/wiki/Multiple_Whonix-Workstation
  https://www.whonix.org/wiki/Connections_between_Gateway_and_Workstation
  https://www.whonix.org/wiki/Whonix-Workstation_to_Whonix-Workstation_Connections
- Qubes networking between qubes:
  https://github.com/QubesOS/qubes-doc/blob/main/user/security-in-qubes/firewall.rst
  https://github.com/QubesOS/qubes-doc/blob/main/developer/system/networking.rst
- onion-grater merger and per-host rules:
  https://github.com/Whonix/onion-grater/blob/master/usr/lib/onion-grater-merger
  https://forums.whonix.org/t/workstation-hardcoded-ip-changes/21595
