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

## T-CLEAN-9: restart, persistence and an independent dial from Ubuntu

Date: 2026-11-16, 12:03 to 13:19 UTC.

The name comes from the owner's brief. This is not the plan's scenario
T-CLEAN-9 (invitation cards), which has still not been run.

Result: **PASS.** After a normal shutdown and restart of both VMs, both
installations reopened their profiles with the same identities, contacts
and messages, and Ubuntu opened its own connection to Debian's onion
service. The first attempt of that connection timed out; Monolith's
automatic retry delivered the message.

| Check | Result |
| --- | --- |
| Both profiles reopened | PASS |
| Identities preserved | PASS |
| Contacts preserved | PASS |
| T-CLEAN-8 messages preserved, no duplicate | PASS |
| Reconnection to each Tor | PASS |
| Independent SOCKS dial from Ubuntu to Debian | PASS |
| Debian's onion service reached from Ubuntu | PASS |
| Message B -> A | PASS |
| Acknowledgement A -> B | PASS, on a new connection from Debian |
| Network isolation of the Monolith process | PASS |
| Snapshot images unchanged | PASS |

The first connection from Ubuntu, classified on its own:

| Property | Result |
| --- | --- |
| Eventual delivery | PASS |
| Automatic recovery | OBSERVED |
| First-attempt success | FAIL for this observation |
| Root cause of the first timeout | UNKNOWN |
| Reliability over repeated cold starts | NOT YET MEASURED |

### Setup

The same two VMs, executable, identities and contacts as in T-CLEAN-8.
Both VMs had been shut down from inside at the end of T-CLEAN-8 and
booted normally from their current disks; no snapshot was restored. This
time the versions were read in the session: Debian's `tor` package
0.4.9.11-0+deb12u1, Tor Browser 16.0.1 with tor 0.4.9.16, and the same
versions in Monolith's Tor settings. The executable's SHA-256 was checked
again in both guests.

### Restart and persistence

- Before Monolith started, the four files of each profile had the
  modification times recorded at the end of T-CLEAN-8.
- Both profiles opened with their lab passphrase: "P9S D12 A" and
  "P9S U24 B", one contact each ("Lab B", "Lab A"), "Tor connected".
- Ubuntu's identity page gave the same pixel hashes, for the region with
  the name and the full fingerprint, as the record of T-CLEAN-6. Debian
  had no such record; its avatar, name and first fingerprint group were
  pixel-identical to the covered record of T-CLEAN-6. On both sides the
  vault, which holds the identity keys and the contacts, was byte for
  byte the same before and after the session. The authenticated sessions
  between the two, each holding the other's identity key from the cards
  of T-CLEAN-8, confirm both identities at protocol level.
- Both T-CLEAN-8 messages were present once each, with unchanged text and
  their delivery marks.
- The Tor settings were unchanged on both sides.

### No session before the test

After the unlock each side opened its Tor control connection, made two
SOCKS connections that carried only the 3-byte greeting (Monolith's Tor
status check), and listened on 127.0.0.1 for its onion service. For the
next ten minutes neither accepted a stream nor sent a CONNECT, and no
stream existed between them: with nothing queued, nothing is dialed
(`RESOURCE_LIMITS.md` section 7). Debian sent nothing before Ubuntu's
message.

### Message B -> A and the first timeout

The message: "P9S-TCLEAN9-B2A-001", an em dash (U+2014), "Independent
connection from Ubuntu.", sent once at 12:39:32 (host clock). The times
below are guest clocks; both guests keep NTP time, and the offset between
them was within about a second.

1. 12:39:30.57: Ubuntu's process connected to its Tor's SOCKS port and
   sent the greeting (3 bytes), its isolation credentials (44 bytes) and
   a CONNECT request of 69 bytes, the length of a version 3 onion name and
   a port. Debian's onion service had been created 11 minutes earlier.
2. No SOCKS success came, and no application data was written. The first
   attempt was observed to end between 114.5 and 120.0 s after the
   CONNECT (`CONNECT_TIMEOUT` is 120 s, `RESOURCE_LIMITS.md` section 4);
   its exact termination time was not captured.
3. 12:41:40.11, after the retry delay: Monolith started a second
   attempt on its own, on a new SOCKS connection, with the same 3, 44 and
   69 bytes.
4. 12:41:40.47: Debian's process accepted the stream on its onion
   service's local listener, 0.35 s after that CONNECT by the two guest
   clocks. Ubuntu wrote its first application data 0.85 s after the
   CONNECT, on its own clock, so the accept fell within that interval.
5. The authenticated session carried the message. By 12:42:22, the first
   screenshot afterwards, Ubuntu showed it as Delivered and Debian showed
   it once, with the exact text, em dash included. Nobody retried,
   restarted or touched anything between the two attempts.

Debian's listener also accepted a stream at 12:41:27.16, about 117 s
after the first CONNECT. That stream carried no data and was closed. It
was probably the first attempt arriving at Debian at the edge of Ubuntu's
timeout; the evidence cannot show which request it belonged to.

What the evidence does not show: why the first attempt took so long. The
descriptor fetch, the introduction and the rendezvous happen inside Tor
and were not observed; the session read no Tor log. Neither descriptor
propagation, circuit construction nor an application defect is claimed
as the cause.

Observed and inferred: the destination of both CONNECT requests is
inferred, not read. The system call trace hides all buffers, so no onion
address was recorded. The inference rests on the request length, on
Debian accepting a stream right after the second request, and on the
session that followed authenticating Debian's identity and carrying the
message.

### Acknowledgement A -> B

"P9S-TCLEAN9-A2B-ACK", an em dash, "Received your independent
connection.", sent once from Debian at 12:44:39 (host clock). By then the
session Ubuntu had opened was gone: a confirmed session with nothing to
deliver is closed after `SESSION_LINGER` (120 s, `RESOURCE_LIMITS.md`
section 4), and Debian's last write on it came at 12:43:42, 120 s after
the last message activity. So Debian dialed:
12:44:37.95 its CONNECT of 69 bytes to Tor's SOCKS port, application data
4.7 s later, and Ubuntu accepted the stream on its onion service's
listener at 12:44:42.21. Debian showed Delivered, Ubuntu showed the text
once. The reply of T-CLEAN-8 had been sent 109 s after the message,
within `SESSION_LINGER`, which is why it used the existing stream then.

### Observation and close

For four minutes with no action, and at the close, each conversation held
exactly the four messages, each once. Both applications exited with
status 0. In each profile only the message store changed, and the lock
was released.

### Network observation

As in T-CLEAN-8: each guest's external interface, each VM's NAT adapter
from the host side, and the Monolith process under `strace` with buffers
hidden, with `ss` samples every five seconds. All four captures covered
all nine windows without a gap: the start, W0 (5 minutes), OPEN, VERIFY,
B2A, A2B, OBS, CLOSE and W5.

- The Monolith process connected only to Tor's SOCKS and control
  endpoints on 127.0.0.1 (on Debian, Tor's control socket), listened only
  on 127.0.0.1, and used the desktop's local sockets. The trace checker
  reported no finding on either side.
- It made no DNS query, and no sample showed it with a socket outside
  loopback.
- Outside loopback: Tor's relay traffic, and the systems' own traffic.
  That was a package download by apt right after the Debian login, before
  Monolith started; time synchronisation, mDNS and IPv6 multicast listener
  reports; on Ubuntu, snapd, NetworkManager's connectivity checks and the
  DNS queries for them. Host and guest captures agree window by window.

### Host and integrity

- The commit charge stayed at 62.6 to 62.8 % for the five minutes before
  Monolith started, and at most 64.2 % in the session (at the VM boots).
  The external disk stayed present and reported no error.
- Both VMs were shut down from inside. Their configurations were as
  before apart from the state time, and the frozen snapshot images had
  the same hashes before and after.
- Two operating slips, neither involving Monolith: Ubuntu's screen locked
  during the observation, and was unlocked with no input reaching
  Monolith; three read-only shell commands typed into the terminal tab
  running Monolith's trace ran there after Monolith exited.

### Not tested in this session

- First-connection reliability over repeated cold starts.
- Delivery to a peer that is offline; an interruption of Tor and the
  recovery.
- File transfers.
- Windows, alone or as a peer.
- Independent physical hosts or networks.
- An external security audit.

### Evidence

Private, on the owner's machine: the folder `D12-U24-TCLEAN9`, 169 files
listed in its `SHA256SUMS.txt` (SHA-256 `effaf513...`).

## T-CLEAN-10: the first connection after a cold start

Date: 2026-11-16, 14:05 to 17:04 UTC.

The name comes from the owner's brief. This is not the plan's scenario
T-CLEAN-10 (five messages each way), which has still not been run as the
plan describes it.

Purpose: after a normal shutdown and start of both VMs, both Tors and both
applications, see whether the first connection from one installation to
the other succeeds, and classify any failure. Three trials: a diagnostic
sample, not a reliability benchmark.

| Property | Result |
| --- | --- |
| Execution | COMPLETE, three trials within the approved bounds |
| Eventual delivery | 3 of 3 PASS |
| First-attempt success | 2 of 3 |
| Automatic recovery | OBSERVED in R3 |
| Network isolation of the Monolith process | PASS within the observed coverage |
| Root cause of the failed first attempt | UNKNOWN |
| Production reliability | NOT ESTABLISHED |

Three trials do not give a failure rate.

### Procedure

The same VMs, identities, contacts and Tor versions as in T-CLEAN-9. The
executable was last hashed in both guests in T-CLEAN-9, earlier the same
day, and was not reinstalled since. For each trial:

- Both VMs were powered on from off; no snapshot was restored. Debian's
  tor ran as the system service; Tor Browser was started and connected.
- The host's commit charge stayed below 90 % for five minutes with both VMs
  and both Tors running (63.4 to 64.9 % in the three trials).
- The guest captures, the host-side traces of the NAT adapters and the
  trace of the Monolith process ran from before Monolith started until
  after it exited, as in T-CLEAN-8 and T-CLEAN-9.
- Both profiles were unlocked. The identity, the contact and every earlier
  message were present, each message once. Neither side accepted a stream
  or sent a CONNECT before the message: with nothing queued, nothing is
  dialed.
- About ten minutes after the unlocks, one message was submitted once,
  followed by five minutes of observation. The only input in that time was
  a key that keeps the screens from locking. Then both applications were
  closed and both VMs shut down from inside.
- Nothing was retried by hand; neither Monolith nor Tor was restarted
  during a trial; no timeout or setting was changed.

### Results

| Trial | Direction | First attempt | Retries | Submit to durable delivery |
| --- | --- | --- | --- | --- |
| R1 | Ubuntu to Debian | PASS, SOCKS success after 20.8 s | none | 22.1 s |
| R2 | Debian to Ubuntu | PASS, SOCKS success after 7.5 s | none | 9.1 s |
| R3 | Ubuntu to Debian | FAIL, ended after 14.9 s | one, automatic | 27.4 s |

"SOCKS success" is the time from the CONNECT request to the first
application data the dialer wrote, which it writes only after a SOCKS
success. In every trial the receiver showed the message once, with the
exact identifier, in the right conversation, and the sender showed it as
Delivered. No duplicate appeared.

- Debian's tor reported its bootstrap complete 60 to 70 s after power-on;
  Tor Browser was connected 2 to 7 s after "Connect".
- Each side had asked its Tor for its onion service (`ADD_ONION`) 10 to
  12 minutes before the first dial.
- Whenever a dial succeeded, the receiver accepted the stream 0.2 to 0.3 s
  before the dialer's SOCKS success, and the message was durable at the
  receiver 1.1 to 1.9 s after that acceptance.

### The failed first attempt of R3

Guest clocks, both synchronized by NTP; the order of events across the two
machines is good to about 0.1 s.

1. Ubuntu's process opened a connection to its Tor's SOCKS port and sent
   the greeting, the isolation credentials and a CONNECT request of 69
   bytes.
2. 14.7 s later Debian's process accepted a stream on its onion service's
   local listener. Neither side wrote application data on that stream,
   and Debian no longer held it 2.2 s after accepting it.
3. 14.9 s after the CONNECT, about 0.2 s after that acceptance, Ubuntu's
   process had closed the SOCKS connection without writing application
   data: the application did not see a SOCKS success. 14.9 s matches no
   timeout of Monolith (`CONNECT_TIMEOUT` is 120 s). Ubuntu then queried
   its Tor's status and showed "Lab A cannot be reached right now ... Next
   try in less than a minute."
4. 11.2 s later Monolith retried on its own, with a new SOCKS connection
   and CONNECT. Debian accepted the stream 0.23 s after that CONNECT, the
   SOCKS success came 0.46 s after it, and the message was durable at
   Debian 1.3 s after it.

Nobody resent the message, and it appeared once.

The first attempt of T-CLEAN-9, in the same direction, was observed to end
between 114.5 and 120.0 s after the CONNECT; its exact termination time
was not captured. There too Debian's listener accepted a stream without
data within a few seconds of the end of that attempt, and the automatic
retry reached Debian in 0.35 s. Both failures seen so far are from Ubuntu
(Tor Browser) to Debian (system tor), and in both a stream reached the
receiver's listener close to the time the dialer gave up, with no
application data on it. They differ in length: 14.9 s, and 114.5 to
120.0 s.

Failure boundary: the dialing application did not see a SOCKS success.
Whether Tor sent a failure reply, closed the connection without one, or
sent a reply the application did not accept is not established: system
call buffers were hidden, receive calls were not traced, Tor Browser keeps
no log by default and Debian's tor logged no warning or error. Not
supported by the evidence: an incomplete Tor bootstrap, a failure of the
connection to the SOCKS port itself, a delay after the transport existed,
host resource pressure. Neither a Tor defect nor a Monolith defect is
claimed.

### Network observation

- All twelve captures (guest and host side, two machines, three trials)
  covered every window of their trial without a gap.
- The trace checker reported nothing in any of the six runs of Monolith.
  The process connected only to Tor's SOCKS and control endpoints on
  127.0.0.1 (on Debian, Tor's control socket), listened only on 127.0.0.1,
  made no DNS query and never held a socket outside loopback.
- One SOCKS CONNECT per attempt, four in all, each after the greeting and
  the isolation credentials.
- Outside loopback: Tor's relay connections, unbroken through the failed
  attempt, and the systems' own traffic (time synchronization, mDNS, IPv6
  multicast listener reports, Ubuntu's connectivity checks and snapd, and a
  package download by apt after login on both systems).

### Host and integrity

- The commit charge was at most 65.8 % (at the VM boots). The external disk
  stayed present and reported no error.
- The frozen snapshot images had the same hashes before R1 and after each
  trial. The VM configurations were unchanged apart from the state time.
  In each profile the vault was unchanged and only the message store
  changed; every exit had status 0 and released the lock.
- The staging copies of the T-CLEAN-8 cards that remained in the guests'
  home folders were deleted during R1, after their hashes were checked.
  That is ordinary deletion, not secure erasure.

### Not established

- A failure rate, or the reliability of the first connection in general.
- The SOCKS outcome of a failed attempt, and the reason for it.
- Whether the failures depend on the direction or on the Tor
  implementation of either side.
- Behaviour on physically independent hosts and networks.

### Evidence

Private, on the owner's machine: the folder `D12-U24-TCLEAN10`, 321 files
listed in its `SHA256SUMS.txt` (SHA-256 `415b9e36...`).

## T-CLEAN-11: a message queued while the recipient is offline

Date: 2026-11-17, 13:38 to 14:42 UTC.

The name comes from the owner's brief. This session is a reduced form of
the plan's scenario T-CLEAN-11: one message instead of three, and about
two and a half minutes with the recipient closed instead of ten.

Purpose: send a message from Debian while Monolith is not running on
Ubuntu, and see whether Debian keeps it queued, does not report it as
delivered, and delivers it on its own once Ubuntu's Monolith is open
again. There is no mailbox in between: the message waits on the sender.

| Property | Result |
| --- | --- |
| Recipient application closed when the message was sent | PASS |
| One message queued and shown as queued | PASS |
| Never shown as Delivered while the recipient was closed | PASS |
| Message store written before the first connection attempt | PASS (observed) |
| Queue kept across a restart of the sender | NOT TESTED |
| Automatic delivery after the recipient returned | PASS |
| One copy, exact text, durable acknowledgement | PASS |
| Network isolation of the Monolith process | PASS within the observed coverage |
| Host memory gate (300 s below 90 % commit) | PASS on the recorded samples; see below |

### Procedure

- The same VMs, identities, contacts and Tor versions as in T-CLEAN-10.
  The executable was hashed in both guests and matched the approved
  build. Both VMs were powered on from off; no snapshot was restored.
- The process trace was the diagnostic variant prepared in the targeted
  Tor diagnostic of the same day: the earlier system call trace plus
  receive and close calls, still without buffer contents, read by an
  analyzer that reports only times, byte counts and outcomes. Its
  self-test passed again in both guests. The guest captures and the
  host-side traces of the NAT adapters ran as before.
- Ubuntu's Monolith was not started until the recipient's return. Tor
  Browser was started, connected, and stayed connected throughout.
- Debian's profile was unlocked. Every earlier message was present, each
  once, every outgoing one Delivered, and Debian made no CONNECT before
  the message: nothing was queued.
- The message: "P9S-TCLEAN11-A2B-OFFLINE-001", an em dash (U+2014),
  "Queued while Ubuntu is offline.", sent once at 14:07:42.8 (host clock).
  About 100 s later Ubuntu's Monolith was started and its profile
  unlocked, followed by five minutes of observation.
- Nothing was retried or resent by hand; neither Monolith on Debian nor
  either Tor was restarted; no timeout or setting was changed. The only
  other input was a key that keeps the screens from locking.

### Clocks

Both guests kept NTP time (offsets of 6 and 16 ms at the start). The host
clock ran about 0.6 to 0.9 s ahead of the guests in this session: the key
press was sent at 14:07:42.8 by the host clock, and Debian began writing
its message store at 14:07:42.218 by its own clock. Times below are guest
clocks. An interval between an event on Debian and one on Ubuntu crosses
the two guest clocks and is good to a few tens of milliseconds.

### While Ubuntu was closed

1. 14:07:42.218: Debian created `messages.new` for exclusive writing, the
   first step of the atomic replacement of its message store
   (`STORAGE.md` sections 3.5 and 5), 33 ms before its first connection
   to its Tor's SOCKS port. The store's modification time changed
   accordingly. The interface showed the message with the queued mark.
2. Four attempts followed. Each sent its CONNECT, received a complete
   10-byte SOCKS reply, wrote no application data and closed: a failure
   reply. They ended 19.1, 4.6, 2.6 and 1.7 s after their CONNECT. The
   reply code is not recorded by the trace.
3. The pauses between the end of one attempt and the next CONNECT were
   7.2, 14.4, 32.4 and 68.8 s, each within the documented range (10, 20,
   40 and 80 s, each times a factor between 0.5 and 1.5;
   `RESOURCE_LIMITS.md` section 7).
4. The interface showed "Lab B cannot be reached right now. Their
   Monolith may be closed or offline. Monolith tries again from time to
   time; your messages stay queued on this device. Messages can be
   delivered only while Monolith is open on both sides." The message was
   never shown as Delivered, and the failed attempts did not rewrite the
   store.
5. Ubuntu's onion service did not exist: its Monolith had not run since
   the VM started. No process, listener or socket of Monolith existed 61 s
   before and 70 s after the message, and its profile was untouched.

### Ubuntu's return and the delivery

1. 14:09:21: Ubuntu's Monolith was started and its profile unlocked. It
   read Tor Browser's control cookie (the SAFECOOKIE preset), and at
   14:10:05.071 its onion service's local listener was ready; the
   interface showed "Tor connected" with the same identity and contact.
   With nothing queued, Ubuntu dialed nothing.
2. 14:10:13.04: Debian's fifth attempt, the retry that had come due 68.8 s
   after the fourth, started 8.0 s after Ubuntu's listener was ready.
   Ubuntu accepted the stream at 14:10:19.04, and Debian's SOCKS success
   came at 14:10:19.36, 6.3 s after the CONNECT.
3. 14:10:20.914: the message was durable in Ubuntu's store.
   14:10:21.299: Debian recorded it as Delivered.
4. Ubuntu showed the exact text once, in the conversation with Lab A;
   Debian showed the same message Delivered. No contact was added on
   either side.

| Interval | Duration | Clocks |
| --- | --- | --- |
| Store write at Debian to durable storage at Ubuntu | 158.70 s | both guests |
| Ubuntu's listener ready to Debian's next CONNECT | 7.97 s | both guests |
| Ubuntu's listener ready to durable storage at Ubuntu | 15.84 s | Ubuntu |
| Ubuntu's listener ready to Delivered recorded at Debian | 16.23 s | both guests |

The time to deliver after the recipient returned depended on where the
sender was in its retry schedule. A contact with nothing queued does not
dial (`RESOURCE_LIMITS.md` section 7), so after a longer absence the wait
could be as long as the sender's current delay, up to 30 minutes times
0.5 to 1.5.

### Persistence: what was observed

- Observed: the message store was rewritten at the submission, through
  the documented atomic replacement, before any connection attempt, and
  not again until the message was Delivered. This agrees with
  `adr/0007-delivery-semantics.md`: a message is stored as Queued before
  the command that queued it returns.
- Not observed: the write, sync and rename calls themselves, which were
  not in the traced set, and the store's content, which is encrypted and
  was not read.
- Not tested: that a queued message survives a restart of the sender's
  application or operating system.

### Network observation

- The four captures (guest and host side, two machines) covered all nine
  windows of the session without a gap.
- The trace checker reported no finding for either run of Monolith. The
  process connected only to its Tor's SOCKS and control endpoints on
  127.0.0.1 (on Debian, Tor's control socket), listened only on
  127.0.0.1, made no DNS query and never held a socket outside loopback.
  The analyzer found no buffer content in either trace.
- Outside loopback: Tor's relay connections and the systems' own traffic
  (time synchronization, mDNS, IPv6 multicast listener reports, Ubuntu's
  connectivity checks and a firmware metadata refresh by fwupd).

### Host and integrity

- Memory gate. The window declared for it ran from 13:56:28 to 14:01:40
  by the host clock, 312 s. The session log recorded 289 s: the span
  from the first to the last of the 20 samples taken inside that window,
  and the gate script accepted a span of 285 s or more, less than the
  required 300 s. The host sampler took a sample every 15 or 16 s.
  Every sample from 13:54:39, the first after Tor Browser was seen
  connected with both VMs running, to 14:01:29, the last before the gate
  was evaluated, showed a commit charge between 65.8 and 66.2 %: 410 s of
  consecutive samples below 90 %. The requirement was met on these
  samples; the gate as evaluated at the time did not itself check 300 s.
- The commit charge was at most 66.9 % (at a VM boot). Paging and disk
  latency peaked only during the boots and logins. The external disk
  stayed present and reported no error.
- The frozen snapshot images had the same hashes before and after. The VM
  configurations were unchanged apart from the state time. In each
  profile only the message store changed; every exit had status 0 and
  released the lock.

### Not established

- That a queued message survives a restart of the sender's application
  or operating system, a crash or a power loss.
- The delay of delivery after a long absence of the recipient.
- The reason for the failed first attempts of T-CLEAN-9 and T-CLEAN-10.
  It did not recur here: the only attempt made while Ubuntu's Monolith
  was open succeeded.
- Behaviour on physically independent hosts and networks.

### Evidence

Private, on the owner's machine: the folder `D12-U24-TCLEAN11`, 157 files
listed in its `SHA256SUMS.txt` (SHA-256 `d0cebf37...`).

## T-CLEAN-12: a queued message across a restart of the sender

Date: 2026-11-17, 15:20 to 16:49 UTC.

The name comes from the owner's brief. The plan's scenario T-CLEAN-12
(Tor interrupted) has not been run.

Purpose: queue a message on Debian while Monolith is not running on
Ubuntu, close Monolith on Debian, shut Debian down and start it again,
and see whether the same message is still queued afterwards and is
delivered on its own once Ubuntu's Monolith is open.

| Property | Result |
| --- | --- |
| Recipient application closed until after the sender's restart | PASS |
| One message queued, not shown as Delivered before delivery | PASS |
| Message store written before the first connection attempt | PASS |
| Message store unchanged across the shutdown and the restart | PASS |
| The same message queued after the restart, once | PASS |
| Automatic delivery after the recipient returned | PASS |
| One copy, exact text, durable acknowledgement | PASS |
| Network isolation of the Monolith process | PASS within the observed coverage |
| Host memory gates (300 s on a monotonic clock) | PASS, 318.9 s and 320.4 s |
| Crash, forced power-off, power loss or interrupted write | NOT TESTED |

### Procedure

- The same VMs, identities, contacts and Tor versions as in T-CLEAN-11.
  The executable was hashed in both guests and matched the approved
  build. Both VMs were powered on from off; no snapshot was restored.
- The diagnostic process trace and the captures as in T-CLEAN-11.
  Debian's observation came in two segments: its guest capture and
  process trace were stopped cleanly before the shutdown and started
  again after the restart, before Monolith. The host-side trace of
  Debian's NAT adapter covered the boot in between.
- The host memory gate used a new rule: samples at most 20 s apart and
  at least 300 s between the first and the last on the host's monotonic
  clock. It ran before the message and again after the restart.
- Ubuntu's Monolith was not started until the recipient's return; Tor
  Browser stayed connected throughout.
- The message: "P9S-TCLEAN12-A2B-REBOOT-001", an em dash (U+2014),
  "Queued before sender reboot.", sent once at 15:52:14.7 (host clock).
- Nothing was retried or resent by hand and no setting was changed. The
  only other input was the unlocks, the close confirmation described
  below and a key that keeps the screens from locking.

### Clocks

Both guests kept NTP time (offsets of 50 and 13 ms). The host clock ran
about 0.8 to 1.0 s ahead of the guests. Times below are guest clocks; an
interval between the two guests is good to about 0.1 s.

### Before the restart

1. 15:52:13.856: Debian created `messages.new` for exclusive writing,
   the atomic replacement of its message store, 183 ms before its first
   connection to its Tor's SOCKS port. The interface showed the message
   with the queued mark.
2. Five attempts each received a complete 10-byte SOCKS failure reply,
   11.6, 1.5, 3.4, 2.1 and 7.9 s after their CONNECT, and wrote no
   application data. The pauses before the next CONNECT were 9.6, 22.4,
   51.6 and 66.4 s, each within the documented backoff
   (`RESOURCE_LIMITS.md` section 7). The message was never shown as
   Delivered.
3. Closing the window asked: "Close Monolith? 1 message has not been
   delivered yet. Monolith tries to send it again the next time it is
   open." After the confirmation the process exited with status 0 and
   released the profile lock. The store had not changed since the
   submission.
4. Debian was shut down from inside, powered off and started again from
   the same disk image. Its tor started as a system service and
   bootstrapped. The boot identifier was new; the installation was the
   same.
5. Before Monolith was started again, the message store had the same
   SHA-256 and modification time as before the shutdown.

### After the restart

1. Monolith was started under the trace and the profile unlocked. The
   conversation showed the same message: same text, same time (15:52),
   the queued mark, once. The store was read and not written.
2. The retry schedule started again: the first attempt 30.6 s after the
   store was read, within the startup spread of 60 s, then pauses of
   9.0, 29.6, 31.2 and 72.8 s. Five attempts failed in the same way while
   Ubuntu's Monolith was still closed. The schedule is not kept across a
   restart, as `RESOURCE_LIMITS.md` section 7 describes.
3. 16:18:04.281: Ubuntu's Monolith, started and unlocked, had its onion
   service's local listener ready. It read Tor Browser's control cookie
   (the SAFECOOKIE preset) and showed "Tor connected" with the same
   identity and contact. With nothing queued, it dialed nothing.
4. 16:19:51.6: Debian's next attempt, due 172.8 s after the previous
   failure. Ubuntu accepted the stream at 16:20:06.50, and Debian's SOCKS
   success came at 16:20:06.84, 15.2 s after the CONNECT.
5. 16:20:08.717: the message was durable in Ubuntu's store.
   16:20:09.126: Debian recorded it as Delivered.
6. Ubuntu showed the exact text once, in the conversation with Lab A;
   Debian showed the original message Delivered. No contact was added.

Eleven attempts in all, every one automatic: ten failed while the
recipient's application was closed, five before and five after the
restart; the eleventh, the only one made while it was open, succeeded at
its first try.

| Interval | Duration | Clocks |
| --- | --- | --- |
| Store write at Debian to durable storage at Ubuntu | 1674.9 s | both guests |
| Ubuntu's listener ready to Debian's next CONNECT | 107.4 s | both guests |
| Debian's CONNECT to its SOCKS success | 15.2 s | Debian |
| Ubuntu's listener ready to durable storage at Ubuntu | 124.4 s | Ubuntu |
| Ubuntu's listener ready to Delivered at Debian | 124.9 s | both guests |

The 1674.9 s include about 26 minutes during which Ubuntu's Monolith was
deliberately closed and Debian was shut down and restarted. Of the
124.4 s after the recipient's return, 107.4 s were the rest of a pause
of Debian's retry backoff, not a timeout; establishing the connection
took 15.2 s, and the message was durable at Ubuntu 1.9 s after the
SOCKS success.

### Persistence: what was shown

- Shown: a queued message survives a graceful exit of the application
  and a normal shutdown and start of the operating system. The store was
  byte for byte the same after the restart, and the application brought
  back the same message and delivered it once.
- Not shown: survival of a sudden power loss, a forced power-off of the
  VM, a crash of the application, damage to the file system or an
  interrupted write.

### Network observation

- Ubuntu's capture ran without a break; Debian's two capture segments
  and the host-side traces of both machines covered every window. No
  Monolith process ran outside the traced intervals.
- The trace checker reported no finding for any of the three runs of
  Monolith. The processes connected only to their Tor's SOCKS and
  control endpoints, listened only on 127.0.0.1, made no DNS query and
  never held a socket outside loopback.
- Outside loopback: Tor's relay connections (Debian's tor used the same
  two guards before and after the restart) and the systems' own traffic
  (time synchronization, mDNS, IPv6 multicast, DHCP and a package
  metadata refresh around Debian's boot and login, Ubuntu's connectivity
  checks).

### Host and integrity

- Both memory gates passed under the new rule: 318.9 s and 320.4 s,
  samples at most 16.6 s apart. The commit charge was at most 67.5 %;
  paging and disk latency peaked only at the boots, the logins and the
  stop of a capture. The external disk reported no error.
- The frozen snapshot images had the same hashes before and after. The
  VM configurations were unchanged apart from the state time and the
  settings of the host-side trace. In each profile only the message
  store changed; every exit had status 0 and released the lock.

### Not established

- Survival of a crash, a forced power-off, a power loss, an interrupted
  write or a damaged file system.
- The delay of delivery after a long absence: the retry schedule starts
  again at each start of the application, and a contact with nothing
  queued does not dial.
- The reason for the failed first attempts of T-CLEAN-9 and T-CLEAN-10.
  It did not recur: the only attempt made while Ubuntu's Monolith was
  open succeeded.
- Behaviour on physically independent hosts and networks.

### Evidence

Private, on the owner's machine: the folder `D12-U24-TCLEAN12`, 212 files
listed in its `SHA256SUMS.txt` (SHA-256 `b8439909...`).

## Memory gates: retrospective review

Added on 2026-11-19. Retrospective: no trial was repeated and no record
was changed.

Each two-machine session required the host's commit charge to stay
below 90 % for 300 consecutive seconds with both VMs and both Tors
running, before Monolith was started. Until T-CLEAN-11 the gate script
counted only the samples inside the declared window and accepted a span
of 285 s or more between the first and the last; the spans it reported
were 288 to 290 s, which do not establish 300 s on their own.

The review reads the original host samples. They were taken every 15 to
17 s by the host's wall clock, written to the second, without a
monotonic clock; a span is good to about 2 s. For each gate it takes
every sample after both Tors were up (Tor Browser seen connected;
Debian's tor had bootstrapped earlier) and up to the end of the declared
window, when the gate was evaluated and Monolith was started. A gate is
supported if no sample shows 90 % or more, a VM not running or the
external disk absent, no two samples are more than 20 s apart, and the
first and last samples are at least 300 s apart.

| Gate | Span inside the declared window | Span from both Tors up | Largest gap | Commit | Result |
| --- | --- | --- | --- | --- | --- |
| T-CLEAN-10 R1 | 290 s | 336 s | 17 s | 63.4 to 63.7 % | PASS, supported by the original samples |
| T-CLEAN-10 R2 | 289 s | 319 s | 16 s | 63.8 to 64.1 % | PASS, supported by the original samples |
| T-CLEAN-10 R3 | 289 s | 304 s | 16 s | 64.5 to 64.9 % | PASS, supported by the original samples (smallest margin) |
| Targeted Tor diagnostic, trial D1 | 288 s | 394 s | 16 s | 65.2 to 65.6 % | PASS, supported by the original samples |
| T-CLEAN-11 | 289 s | 410 s | 16 s | 65.8 to 66.2 % | PASS, supported by the original samples |

The trial D1 belongs to a diagnostic session of 2026-11-17 that is not
otherwise reported in this file. The outcomes and limitations of the
trials themselves are unchanged. From T-CLEAN-12 on, the gate measures
the span on the host's monotonic clock and requires 300 s.
