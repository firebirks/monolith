# Clean-machine test results (P9-S)

This file records the sessions run under the clean-machine test plan
(`CLEAN_MACHINE_TEST.md`). The results do not change the plan or its
acceptance criteria; where a session departed from the plan, this file
says so.

The evidence of each session (session log, screenshots, packet captures,
system call traces, host records) stays on the owner's machine, as section
9 of the plan requires. This file summarises it. It holds no card or
invitation card, onion address, fingerprint, key, Tor cookie, passphrase or
password, relay address or resolver address.

Session names follow the owner's briefs. They do not match the scenario
numbers of the plan one to one; each session says which scenarios of the
plan it covered.

## T-CLEAN-8: first exchange between two installations over public Tor

Date: 2026-11-15, 17:04 to 19:04 UTC.

Result: **PASS, with independent reverse dialing not yet tested.** The
reply from Ubuntu to Debian travelled on the connection that Debian had
opened. Ubuntu made no connection to Debian's onion service, so this
session does not show that Debian's onion service can be reached from
another installation.

### Setup

| | A | B |
| --- | --- | --- |
| Machine | VM `P9S-D12` | VM `P9S-U24` |
| System | Debian 12.15, GNOME on Wayland | Ubuntu 24.04.5 LTS, GNOME on Wayland |
| VM | 4 GB, 2 vCPU, NAT | 4 GB, 4 vCPU, NAT |
| Tor | Debian's `tor` package, 0.4.9.11 | Tor Browser 16.0.1, tor 0.4.9.16 |
| Monolith's Tor settings | defaults: SOCKS 127.0.0.1:9050, control socket `/run/tor/control` | preset "Tor Browser": SOCKS 127.0.0.1:9150, control 127.0.0.1:9151, cookie |
| Identity | "P9S D12 A", made in T-CLEAN-5 | "P9S U24 B", made in T-CLEAN-5 |
| Local name given to the peer | "Lab B" | "Lab A" |

- Both machines ran the same executable, `monolith-desktop` with SHA-256
  `ca60b4fbb5cf87dcfef25725dbd1ccf39bb56844ee8f94441bc2bb0d15c9a478`,
  installed from the Linux archive with SHA-256
  `f0b612d6a31af57d844b94ccb4df1d8830a4066a786877d19c4cb3a1e39aea23`. The
  build record gives commit `3777a27` for that archive; the commits after
  it up to `596000b` change only documentation. The executable was hashed
  again in both guests at the start of the session. Nothing was rebuilt.
- The Tor versions are those read in T-CLEAN-5 and T-CLEAN-6 (2026-11-13
  and 2026-11-14, in Monolith's Tor details). This session did not read
  them again. Debian's 0.4.9.11 is older than the 0.4.9.13 that
  `TOR_INTEGRATION.md` recommends, and Monolith says so.
- Both VMs ran on one Windows 11 host under VirtualBox, each behind its own
  NAT adapter, and both reached the Internet through the same connection
  and public address. They are two installations, not two independent
  hosts or networks.
- Files reached a VM only on read-only ISO images and left it only on a
  small virtual disk, read on the host without mounting. No shared folder,
  shared clipboard, drag and drop, USB passthrough or guest additions.

### How two installations become contacts

The brief of the session asked for an invitation from A, accepted by B.
Monolith has no separate "send invitation" step for two people who
exchange plain cards (`PROTOCOL.md` sections 6.4 and 12,
`DESIGN_QUESTIONS.md` P6-6):

1. Each side saves its plain card ("Share my card", "Save card") and
   imports the other's ("Add contact"). Each now holds the other as
   requested. Nothing is sent: a contact with nothing queued is not dialed
   (`RESOURCE_LIMITS.md` section 7).
2. The first message queued on one side starts a dial. On that session the
   sender sends ContactRequest, because it holds the peer as requested.
3. The receiver also holds the sender as requested. It accepts the request
   without asking its user and sends ContactAccept (section 6.4). The
   session is confirmed and the message is delivered.

The session followed this, as the owner decided before it started: plain
cards both ways, no invitation card, one message A -> B that carried the
request, one reply B -> A. Invitation cards, which carry a capability
(plan scenario T-CLEAN-9), were not used.

### What was done

1. Check of the transfer method in the reverse direction, U24 -> host ->
   D12, with a synthetic 72-byte marker file: the same SHA-256 in the
   source guest, on the host and in the destination guest, which mounted
   the ISO read-only. The direction D12 -> U24 had been checked on
   2026-11-14.
2. Card export: on each side "Save card" wrote
   `~/Downloads/monolith-card.txt`, mode 0600: 284 characters,
   `MONOLITH1:` followed by base32, no trailing newline. On the host each
   was decoded without being printed: a 171-byte version 1 card with one
   endpoint and no capability.
3. Card transport: each card on its own new 4 MiB virtual disk, not the
   one of step 1. The disk was attached to the source VM while it was off,
   and written in the guest with `dd` once the target device had been
   identified by its size, its serial and a label the host had written.
   The host read the card from the disk and put it on its own read-only
   ISO for the peer. `sha256sum -c` passed in the destination guest. Card
   A `a3123394...`, card B `921b04f2...`. The card text was never shown on
   screen, typed or put on a command line. Screenshots of the card dialogs
   were pixelated or covered before they were saved.
4. Import, first on A, then on B: "Add contact", the path on the ISO typed
   into the path field, "Open", a local name, "Add contact". Each side
   showed "Contact added" with "This is their card, not an invitation. Lab
   B must add your card too before your messages can arrive." ("Lab A" on
   B), and listed the contact as "Waiting for them to add your card".
5. One message from A: "P9S-TCLEAN8-A2B-001", a line break, "Hello from
   Debian.".
6. One reply from B: "P9S-TCLEAN8-B2A-001", a line break, "Hello from
   Ubuntu.".
7. Four minutes with no action, then both applications closed normally.

### Results

| Check | Result |
| --- | --- |
| Reverse synthetic transfer U24 -> host -> D12 | PASS |
| Host memory gate | PASS |
| Card exchange, both directions, hashes end to end | PASS |
| Contact request with the first message, automatic acceptance | PASS |
| B's onion service reached from A | PASS |
| Message A -> B | PASS |
| Reply B -> A | PASS |
| Application traffic through Tor's SOCKS port | PASS on A; B made no dial |
| Network isolation of the Monolith process | PASS |
| Independent dial B -> A, A's onion service reached from B | NOT TESTED |

### Message A -> B

- Send pressed at 18:25:11. A's process connected to 127.0.0.1:9050 and
  wrote, on that one connection, a SOCKS5 greeting (3 bytes), the username
  and password of its isolation credentials (44 bytes) and a CONNECT
  request of 69 bytes: a 62-byte host name, the length of a version 3
  `.onion` name, and a port. Its first application data followed 5.2 s
  later, after Tor's reply.
- About 5 s after the CONNECT, B's process accepted one connection on the
  local listener of its onion service, from B's Tor. It was the only
  inbound connection of the session.
- Within 7 s of the send, the vault of each profile, which holds the
  contacts, was rewritten. At the first screenshot afterwards, 34 s after
  the send, B showed the message in the conversation "Lab A" with both
  lines exact; A showed it as Delivered (two ticks); the waiting notice had
  gone on both sides. B had asked its user nothing and listed no request.
- Before Delivered, A showed the message as Queued (a clock mark).
  Delivered comes from MessageAck (`PROTOCOL.md` section 8.6): B had
  validated the message and stored it durably. MessageAck says nothing
  about display or reading; that B displayed the message is shown by the
  screenshot.

### Reply B -> A

- Send pressed at 18:27:00. B wrote the reply on the connection that A had
  opened at 18:25. B's process made no SOCKS connection after the two
  status checks that follow the unlock.
- At 18:27:10 A showed the reply in the conversation "Lab B" with both
  lines exact; B showed it as Delivered.

### Duplicates and retries

During the four minutes of observation, and at the close, each
conversation held exactly the two messages, each once. Each identity had
one contact and no pending request. Each message was sent once. Nothing was
sent again, no dial failed and Tor was not restarted.

### Onion services and SOCKS

- Tor accepted B's onion service when the profile was unlocked (the
  identity showed "Available"). The upload of its descriptor was not
  observed directly.
- That B's service could be reached from another installation through the
  public Tor network is shown by the chain above: A's SOCKS request for an
  onion name, B's inbound connection from its Tor about 5 s later, and the
  authenticated session that carried the request, the acceptance, the
  message and its acknowledgement.
- Application traffic through SOCKS is shown on A only. B's reply reused
  A's connection; B never asked its Tor for a connection to A. Whether A's
  onion service can be reached from another installation is still open.

### Network observation

Monitored for the whole session, from before Monolith started to after it
exited:

- each guest's external interface (`tcpdump`, headers only, 128 bytes per
  packet, no capture of loopback);
- each VM's NAT adapter from the host side (VirtualBox network trace from
  power-on to power-off, cut to 128 bytes per packet), which the guest
  cannot influence;
- the Monolith process under `strace` with system call buffers hidden
  (`-s 0`), and a root `ss` sample of every socket and its process every
  five seconds.

Windows: the start of the captures, W0 (Tor running, Monolith closed; 5
minutes, shorter than the 15 of the plan), OPEN, IMPORT, A2B, B2A, OBS,
CLOSE and W5. All four captures covered all nine windows without a gap.

- The Monolith process connected only to Tor's SOCKS and control endpoints
  on 127.0.0.1, or on Debian to Tor's control socket, listened only on
  127.0.0.1 for its onion service, and used the local sockets of the
  desktop (Wayland, X11, D-Bus). On Debian, two failed calls to nscd
  followed by reads of the user database were noted, as section 10 of the
  plan allows. The trace checker reported no finding on either side.
- The Monolith process made no DNS query, and no sample showed it with a
  socket to an address outside loopback.
- Outside loopback, each VM sent Tor's traffic to its two relays and the
  system's own traffic: on Debian time synchronisation, mDNS and IPv6
  multicast listener reports; on Ubuntu time synchronisation,
  NetworkManager's connectivity checks, snapd, and the system's DNS
  queries for them. Apart from Tor's relay traffic, no packet went between
  the two VMs.
- The host-side and guest-side captures agree window by window.
- The two earlier runs that exported the cards were traced with `strace`
  (no finding) but not captured.

### Host and integrity

- The host runs other work. Before any real card was handled, and again
  before the import, the commit charge stayed below 90 % for five
  consecutive minutes (79.5 to 82.4 %, then 83.6 to 84.1 %). The highest
  value during the session was 88.8 %, caused by a host workload outside
  the VMs; the 95 % and 97 % limits were never reached. The external disk
  that holds the VMs stayed present and reported no error.
- Afterwards both VMs were shut down from inside. Their configurations
  were as before, apart from the state time, and every test medium was
  detached. The frozen images of the snapshots were hashed before and
  after the session, with the same result.
- In both profiles only the vault and the message store changed. The lock
  was released when each process exited.
- The temporary card disks and ISOs were deleted once the imports were
  verified. That is ordinary file deletion, not secure erasure.
- Two operating slips, neither involving Monolith: Ubuntu's screen locked
  during the observation, and a key meant to close Monolith reached the
  lock screen; the first start of Debian's capture was mistyped, refused
  by the script and then started correctly. A DNS marker query typed with
  it was seen by the host-side trace only.

### Not tested in this session

- A dial from B to A, and the reachability of A's onion service from
  another installation.
- The upload and propagation of an onion service descriptor, observed
  directly.
- Independent physical hosts or networks.
- Delivery to a peer that is offline; a restart of either side and the
  reconnection; an interruption of Tor and the recovery.
- File transfers.
- Windows, alone or as a peer.
- An external security audit.

Scenarios of the plan: T-CLEAN-8 was run, except that the owner did not
compare the fingerprints aloud; the cards' integrity was shown by their
hashes instead. T-CLEAN-9 (invitation cards, a second identity,
revocation) was not run. T-CLEAN-10 was run in part: one two-line message
each way instead of five, none in a non-Latin script.

### Evidence

Private, on the owner's machine: the folder `D12-U24-TCLEAN8`, 228 files
listed in its `SHA256SUMS.txt` (SHA-256 `c5497b12...`).
