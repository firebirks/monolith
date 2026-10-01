# Tails integration

Files for running Monolith on Tails. The design is in
`docs/PLATFORM_TAILS.md`.

| File | Installed as | Purpose |
| --- | --- | --- |
| `onion-grater/monolith.yml` | `/etc/onion-grater.d/monolith.yml` | Control port filter profile |

Status: draft, not yet tested on Tails.

Installing the profile needs an administration password, which has to be set
at the Welcome Screen. The installation does not survive a reboot.

    sudo install -m 0755 monolith /usr/bin/monolith
    sudo install -m 0644 onion-grater/monolith.yml /etc/onion-grater.d/monolith.yml

The profile matches the executable path `/usr/bin/monolith` and the user
`amnesia`. A binary at another path is refused by the filter.
