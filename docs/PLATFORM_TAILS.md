# Tails

Tails is a primary target. Monolith on Tails uses the Tor that Tails runs,
goes through Tails' control port filter, and by default keeps nothing after
shutdown.

Status: not final, and blocked. Nothing here has been run on Tails. The
central assumption, that Monolith can create an Onion Service through
Tails' filtered control port, has not been tested and is contradicted by a
statement in Tor's own manual (section 3.3). Section 3.4 lists what has to
be verified by experiment on a current supported Tails release. Those
checks are a precondition for Phase 3 (Tor integration) and Phase 6
(Tails): neither starts before the results are recorded in this document.

Statements about Tails come from the sources in section 9, read on
2026-10-01. Configuration quotes are from the Tails source at tag 7.11; the
current release is 7.14 and its release notes list no change to networking,
the firewall or onion-grater. Items that could only be inferred are marked
"unverified".

## 1. Platform facts

- Tails 7.14 (2026-09-30), based on Debian 13, GNOME 48 on Wayland. Since
  7.12 there is a new release every two weeks.
- tor 0.4.9.13, installed from deb.torproject.org.
- The live user is `amnesia` (uid 1000).

Tor listeners relevant to Monolith:

| Listener | Purpose | Reachable by `amnesia` |
| --- | --- | --- |
| `127.0.0.1:9050` SOCKS, `IsolateDestAddr IsolateDestPort` | default system-wide SocksPort | yes |
| `127.0.0.1:9052` control port, cookie auth | real control port | no, rejected by the firewall |
| `127.0.0.1:951` | onion-grater, the control port filter | yes |

`IsolateSOCKSAuth` is a Tor default and applies to port 9050 as well, so
Monolith's per-contact SOCKS credentials take effect there.

Note that the filtered control port on Tails is 951, not the conventional
9051, and that Tor's own control port is 9052.

## 2. Network behavior of the platform

The Tails firewall (ferm, iptables) applies these rules to `amnesia`:

- Loopback TCP to any local port is allowed, except a short blocklist that
  includes the real control port.
- Since Tails 7.9, outgoing TCP to public IPv4 addresses is redirected to
  Tor's TransPort. A direct connection is torified, not leaked and not
  blocked. Tails' user documentation still says "blocked".
- Connections to private address ranges (10/8, 172.16/12, 192.168/16) go out
  directly, without Tor.
- UDP to the Internet, ICMP and IPv6 are rejected.
- Other Unix users are rejected everywhere unless explicitly listed.

Consequences for Monolith:

- Monolith must run as `amnesia`. A dedicated service account would have no
  network access at all.
- Monolith's invariants do not rely on the Tails firewall. Monolith opens
  connections to exactly two loopback endpoints (SOCKS 9050 and the filter
  on 951) and one loopback listener. It never connects to a LAN address,
  which on Tails would bypass Tor.
- "No clearnet fallback" cannot be demonstrated on Tails by observing that a
  direct connection fails, because Tails would torify it. The proof is done
  in a network namespace on a development machine (T-NET-1). On Tails the
  check is that the firewall's reject log stays empty and that Monolith
  holds no socket other than the three above.

Inbound: Tor (user `debian-tor`) may connect to any local port, and loopback
input is accepted, so an `amnesia` process listening on `127.0.0.1` receives
Onion Service traffic. This is how OnionShare works on Tails. The input
policy drops everything that is not loopback, so the listener is not
reachable from the LAN.

## 3. Control path

    monolith (amnesia)
        -> 127.0.0.1:951            onion-grater
        -> 127.0.0.1:9052           Tor control port, cookie held by the filter
        -> ADD_ONION ... Port=29170,29170
    peer -> Tor -> 127.0.0.1:29170  Monolith listener

What Monolith sees on port 951:

- `PROTOCOLINFO` is answered by the filter: `AUTH METHODS=NULL` and the real
  Tor version.
- `AUTHENTICATE` with no argument is answered `250 OK` by the filter.
- A command that matches no rule gets `510 Command filtered`.
- The filter opens one upstream control connection per client connection.
  Because Tor scopes non-detached services to the control connection that
  created them, Monolith can only remove its own service, and its service
  disappears when Monolith's connection closes.

### 3.1 How the filter picks a profile

For loopback clients onion-grater finds the client process by its source
port and matches on:

- `apparmor-profiles`: the AppArmor profile name of the process, or the path
  of its executable if it has no profile;
- `users`: the Unix user.

Exactly one profile must match; with none the client gets an empty rule set.
Profiles in `/etc/onion-grater.d/` are read on every new client connection,
so adding one needs no service restart.

A Rust binary without an AppArmor profile is matched by executable path. The
profile therefore names `/usr/bin/monolith` and user `amnesia`.

The Tails source describes this matching as racy and inherently insecure,
and Tails is moving applications into network namespaces matched by address
instead. Monolith's profile will have to follow whatever mechanism Tails
uses at the time of integration.

### 3.2 Profile

`integrations/tails/onion-grater/monolith.yml` is a draft. It allows:

- `GETINFO status/circuit-established`
- `ADD_ONION` in the two exact shapes of `TOR_CONTROL_SURFACE.md`, with
  virtual port 29170 and target port 29170 on loopback
- `DEL_ONION` with a 56-character service id

It allows no `SETCONF`, no `GETCONF`, no `SIGNAL`, no other `GETINFO` key and
no event. The OnionShare profile passes `HS_DESC` events; this one does
not, because Tor delivers them for every Onion Service on the system and
they would show Monolith the addresses of services other applications
created. Monolith therefore does not learn when its descriptor has been
uploaded and shows the service as published without that confirmation.

The listener port is fixed at 29170 because the profile has to name it. On
systems without a filter the port is chosen by the operating system.

### 3.3 Sandbox

Tails runs Tor with `Sandbox 1`. The tor manual says that launching Onion
Services through the control port is not supported with the syscall sandbox.
OnionShare on Tails appears to create its services exactly this way and is
a shipped feature. Which of the two statements describes current Tails is
not known. It has to be found out by experiment, not by reading.

### 3.4 Preconditions to verify on Tails

To be done on a current supported Tails release, before Phase 6 and before
Monolith claims to work on Tails. They do not gate the generic system Tor
backend of Phase 3 (`DESIGN_QUESTIONS.md` T3-1). Each result is recorded here with the Tails version
and date.

1. Whether `ADD_ONION` works through the filtered control interface on port
   951 while Tor runs with `Sandbox 1`.
2. How the current OnionShare publishes its Onion Services in that same
   environment: which commands it sends, through which path, and what Tor
   answers.
3. Whether OnionShare gets integration that an ordinary third-party package
   cannot have: its own network namespace, firewall rule, AppArmor profile,
   a profile shipped in the image, or anything in Tor's configuration.
4. The exact onion-grater profile Monolith needs, starting from the draft in
   `integrations/tails/` and corrected against what the filter accepts.
5. Whether a production-quality installation requires inclusion in Tails or
   at least a Debian package, or whether a documented per-session
   installation is as far as a third-party package can go.
6. For any workaround that comes up: whether it weakens Tor's sandbox or
   changes Tails' Tor configuration.

Rules for this work:

- Tor's sandbox is not disabled to make Monolith work.
- Tails' global Tor settings are not modified.
- No instruction in Monolith's documentation tells a user to do either.
- If the current Tails architecture does not let an ordinary third-party
  package create an Onion Service, this document says so plainly, and
  Monolith on Tails is then limited to what Tails supports until that
  changes upstream. An honest limitation is preferred to an unsafe
  workaround.

Status of each item: not verified.

## 4. What is needed, by situation

### 4.1 Development testing

Requirements: an administration password set at the Welcome Screen (it
cannot be set later).

    sudo install -m 0755 monolith /usr/bin/monolith
    sudo install -m 0644 monolith.yml /etc/onion-grater.d/monolith.yml

No firewall change and no service restart are needed. Everything is gone
after a reboot, because changes outside the Persistent Storage live in RAM.

Running the binary from another path does not work with the shipped
profile: the profile matches on the executable path. For a development
build, edit the path in a local copy of the profile.

Caveat: path matching does not authenticate the program. Any process of
`amnesia` that can run or imitate the binary at that path gets the same
control port rights. The rights are limited to creating Onion Services, in
any number, that point at local port 29170, which is what makes this
acceptable.

### 4.2 Local installation that survives reboots

Tails has no supported way for a third-party application to install a
control port profile persistently.

- Additional Software installs Debian packages through APT and reinstalls
  them at every start. It is documented for packages from Debian's official
  repositories; other sources are an explicit non-goal of Tails. A Monolith
  .deb that ships the binary and the profile, served from a local APT
  repository kept in the Persistent Storage, would work mechanically.
  Unverified, and not supported by Tails.
- Dotfiles only creates symlinks inside the home directory. It cannot place
  a file under `/etc`.
- A custom line in `persistence.conf` can make a directory under `/etc`
  persistent. Tails documents this for APT sources only. Using it for
  `/etc/onion-grater.d` is unverified, would also persist the directory's
  other profiles across Tails upgrades, and is not recommended.

Until Monolith is in Debian, a persistent local installation is an
unsupported setup, and the documentation will say so plainly. The
recommended way to use an unreleased Monolith on Tails is 4.1, repeated per
session, with the persistent vault (section 5) kept in `~/Persistent`.

### 4.3 Inclusion in Tails

Tails requires software to be in Debian first and adds new software only
with a strong reason. A proposal would have to be made on the Tails GitLab.
Inclusion would mean, on the Tails side:

- the onion-grater profile under `/etc/onion-grater.d/`;
- an AppArmor profile, so the filter matches by profile name;
- possibly a network namespace and firewall entry, following OnionShare;
- a Persistent Storage feature for Monolith's data directory.

Monolith's side of that work: Debian packaging, a stable control command
shape, and a single well-defined data directory.

## 5. Persistence

Default on Tails: ephemeral. The identity, the Onion Service key, contacts
and messages exist in memory and are gone at shutdown. The interface shows
`EPHEMERAL SESSION` for the whole session. This is a normal mode of
operation, not an error.

- In ephemeral mode Monolith writes nothing to disk. It does not create a
  configuration directory, a cache or a log file.
- A persistent identity exists only if the user asks for one and names
  where to put it. Monolith then creates the vault there.
- The location must be inside the Persistent Storage. Monolith checks that
  the path resolves under `/home/amnesia/Persistent` (the bind mount of
  `/live/persistence/TailsData_unlocked/Persistent`) and that this directory
  is a mount point. If the Persistent Storage is not unlocked, or the path
  is elsewhere, Monolith refuses and explains that the location would not
  survive shutdown. It does not silently create an amnesic "persistent"
  identity.
- The Dotfiles feature is not suitable for the vault. It symlinks existing
  files only; a file replaced by write-and-rename would land in the amnesic
  home directory.
- Message history stays off unless the user enables it explicitly, and it
  can only be enabled together with a persistent vault.
- The vault is encrypted with its own passphrase even though the Persistent
  Storage is itself encrypted (`STORAGE.md`).
- Tails provides no documented interface for third-party applications to
  ask whether a path is persistent. The check above uses the documented
  mount layout.

What "gone at shutdown" means on Tails: session data lives in RAM. Tails
overwrites most RAM at shutdown and zeroes freed memory, with documented
gaps (some kernel memory, video memory, removal of the boot medium during
suspend). Monolith does not claim more than Tails does. Tails uses no swap.

## 6. Other platform rules

- No second Tor. Monolith has no code to start one (S4).
- No update check and no telemetry (S32). Updates come with the package.
- Platform detection: `ID="tails"` in `/etc/os-release`. Older markers such
  as `/etc/amnesia/version` no longer exist.
- GUI: GNOME 48 on Wayland with XWayland available. GTK 4.18 and libadwaita
  1.7 are in the image; so is Qt 6.8. See ADR 0006.

## 7. Open items

T1. The preconditions of section 3.4. Blocking for Phase 3 and Phase 6.
    Beyond the basic question they include whether `ADD_ONION` with
    `MaxStreams`, `MaxStreamsCloseCircuit` and `PoWDefensesEnabled` is
    accepted.

T2. Confirm that onion-grater matches an unconfined `/usr/bin/monolith` by
    executable path, and that the draft profile parses and behaves as
    intended.

T3. Decide whether Monolith ships an AppArmor profile of its own before
    asking for inclusion.

T4. Track the Tails work on namespace-based matching (their issue 18123) and
    adapt the profile.

T5. Debian packaging, which is the precondition for 4.2 and 4.3.

## 8. Test matrix

See `TEST_PLAN.md` section 9.

## 9. Sources

Accessed 2026-10-01.

- Release: https://tails.net/news/version_7.14/
- Package list: https://tails.net/torrents/files/tails-amd64-7.14.packages
- Design, Tor enforcement and network filter:
  https://tails.net/contribute/design/
  https://tails.net/contribute/design/Tor_enforcement/
  https://tails.net/contribute/design/Tor_enforcement/Network_filter/
- Stream isolation: https://tails.net/contribute/design/stream_isolation/
- Application isolation: https://tails.net/contribute/design/application_isolation/
- Persistent Storage: https://tails.net/contribute/design/persistence/
  and https://tails.net/doc/persistent_storage/configure/
- Additional Software: https://tails.net/doc/persistent_storage/additional_software/
  and https://tails.net/contribute/design/additional_software_packages/
- Memory erasure: https://tails.net/contribute/design/memory_erasure/
- Administration password:
  https://tails.net/doc/first_steps/welcome_screen/administration_password/
- Software inclusion policy: https://tails.net/support/faq/
- Tails source at tag 7.11, read through the Software Heritage archive of
  https://gitlab.tails.boum.org/tails/tails.git (revision
  834df0067039b9102723d59343895f8c18beb6a6). The Tails GitLab required
  sign-in on the access date. Files used: `etc/tor/torrc`,
  `etc/ferm/ferm.conf`, `usr/local/lib/onion-grater`,
  `etc/onion-grater.d/onionshare.yml`, `auto/config`.
- tor manual, Sandbox option:
  https://gitlab.torproject.org/tpo/core/tor/-/blob/tor-0.4.9.13/doc/man/tor.1.txt
