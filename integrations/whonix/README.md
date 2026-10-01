# Whonix integration

Files for running Monolith on Whonix. The design is in
`docs/PLATFORM_WHONIX.md`.

| File | Machine | Installed as | Purpose |
| --- | --- | --- | --- |
| `onion-grater/40_monolith.yml` | Gateway | `/usr/share/doc/onion-grater-merger/examples/40_monolith.yml` | Control port filter profile, opt-in |
| `firewall/40_monolith.conf` | Workstation | `/etc/whonix_firewall.d/40_monolith.conf` | Opens the listener port |

Status: draft, not yet tested on Whonix.

Whonix 18 allows `sudo` only in the SYSMAINT session. Boot the Gateway and
the Workstation into it for the steps below.

Gateway:

    sudo install -m 0644 onion-grater/40_monolith.yml \
        /usr/share/doc/onion-grater-merger/examples/40_monolith.yml
    sudo onion-grater-add 40_monolith

Workstation:

    sudo install -m 0644 firewall/40_monolith.conf \
        /etc/whonix_firewall.d/40_monolith.conf
    sudo whonix_firewall

The profile applies to every Workstation behind the Gateway. It lets a
Workstation create Onion Services that point at its own port 29170 and
nothing else.
