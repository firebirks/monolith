# Whonix integration

Files for running Monolith on Whonix. The design is in
`docs/PLATFORM_WHONIX.md`.

| File | Machine | Installed as | Purpose |
| --- | --- | --- | --- |
| `onion-grater/40_monolith.yml` | Gateway | `/usr/share/doc/onion-grater-merger/examples/40_monolith.yml` | Control port filter profile, opt-in |
| `firewall/monolith-listener-rule` | Workstation | `/usr/libexec/monolith/monolith-listener-rule` | Accepts the listener port from the Gateway only |
| `firewall/whonix-firewall.service.d/40_monolith.conf` | Workstation | `/usr/lib/systemd/system/whonix-firewall.service.d/40_monolith.conf` | Runs the rule script after the firewall loads |

Status: draft, not yet tested on Whonix. The isolation of the listener from
other Workstations is a plan until it has been tested with one Gateway and
two Workstations.

Whonix 18 allows `sudo` only in the SYSMAINT session. Boot the Gateway and
the Workstation into it for the steps below.

Gateway:

    sudo install -m 0644 onion-grater/40_monolith.yml \
        /usr/share/doc/onion-grater-merger/examples/40_monolith.yml
    sudo onion-grater-add 40_monolith

Workstation:

    sudo install -D -m 0755 firewall/monolith-listener-rule \
        /usr/libexec/monolith/monolith-listener-rule
    sudo install -D -m 0644 firewall/whonix-firewall.service.d/40_monolith.conf \
        /usr/lib/systemd/system/whonix-firewall.service.d/40_monolith.conf
    sudo systemctl daemon-reload
    sudo systemctl restart whonix-firewall

Do not add port 29170 to `EXTERNAL_OPEN_PORTS`. That opens it to every
Workstation behind the Gateway.

The onion-grater profile applies to every Workstation behind the Gateway. It
lets a Workstation create Onion Services that point at its own port 29170
and nothing else.

The firewall rule uses a mechanism that Whonix documents as unsupported.
After a manual `sudo whonix_firewall` reload the rule is gone and the port
is closed until `whonix-firewall` is restarted.

On a non-Qubes Gateway shared with other Workstations, a compromised
Workstation can impersonate the Gateway on the internal network and get past
any filter by source address. Use Qubes-Whonix, or give Monolith a Gateway
of its own, if other Workstations are not trusted.
