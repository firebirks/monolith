# Private Tor network test

`two-node.sh` is the end-to-end test of Phases 3 to 8 on a private Tor
network built with Chutney, the Tor Project's tool for local test
networks:

    Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B

Phase 3, with `dev-chat` and identities in memory:

1. B publishes a v3 Onion Service; so does A, so that its card names an
   endpoint. Each receives the other's contact card through a file. A
   connects through its SOCKS port; the Noise XK handshake authenticates
   B to A, then A to B. A sends "hello", B receives it and replies "hello
   back", A receives the reply. Both close the session and remove their
   service with `DEL_ONION`.
2. A service whose control connection went away is published again from
   the key held in memory: it has the same name and is reached again. A
   dial without its SOCKS endpoint fails (`tests/tor_network.rs` in
   `monolith-core`, ignored unless this script sets the endpoints).

Phase 4, with `dev-node`, a node over a persistent vault driven through
its standard input:

4. Each node makes an identity in its vault and imports the other's card;
   one session makes both accepted contacts through the protocol, with
   "hello" and "hello back".
5. B restarts: its identity, its card, its service and its contact are as
   before, and A reaches it again.
6. A rotates its transport key. B holds the announced successor across a
   restart, then promotes it when A dials with the new key; B withdraws
   the session of the old key, and keeps the retired key across another
   restart. A switches and finishes the rotation.
7. B blocks A while their session is open: the session ends, and A's next
   session gets what a stranger gets, and no reply.
8. B makes a second identity in invitation mode: a request without a
   capability is dropped, one with a capability is queued and accepted;
   the two identities hold A differently; revoking the capability leaves
   the contact; deleting the contact withdraws its open session.
9. Tor B is restarted while a session is open, `RESTART_CYCLES` times
   (3 unless set), each time after the services ran longer than the
   supervisor's `RECONNECT_RESET_AFTER`: B reports both services
   unavailable, its supervisor publishes them again under the same names
   and the same card, and A completes Noise XK with B again. A dial that
   Tor A's SOCKS endpoint refuses is sent again; any other failure, or
   no session within `RECOVERY_WINDOW` seconds of the first try (240
   unless set), fails the test. Tor may hold the first stream after the
   restart for its `SocksTimeout` of 120 seconds and then refuse it with
   0x01; the window allows two such waits, so a refusal that persists
   fails. Each cycle prints its attempts, its time, the dial retries of
   the node and the SOCKS reply codes, read from the trace of node A; a
   cycle slower than 10 seconds also prints what A and B logged.
10. A without its SOCKS endpoint, and A without any Tor while the machine
    has direct Internet access: every dial fails at once, and nothing is
    contacted.
11. Every node run is checked: loopback and Unix sockets only, no DNS.

Phase 5, with `dev-runtime`, a node on the messaging runtime over a
persistent vault and its message store, driven through its standard input
(steps 13 to 20; each prints how long its deliveries took):

13. Two runtime nodes each make an identity and import the other's card.
    Nothing is queued, so no session is made. B goes away.
14. A queues "first" while B is away: it is reported queued, and the
    attempt to reach B is deferred. B comes back; B stores "first" once,
    A records it delivered once B's acknowledgement came, and both list it.
15. B, started with `--ack-delay 3600`, stores "second" and is killed with
    SIGKILL before it acknowledges it. A sends it again to the restarted B,
    which acknowledges it without storing it again: B holds "first" and
    "second", once each.
16. A queues "third" while B is away and is killed with SIGKILL. The
    restarted A delivers it once B is back; "first" and "second" are still
    delivered.
17. Tor A is stopped: A reports its service unavailable, queues "fourth",
    and its attempts are deferred for Tor. Tor A is started again: A
    publishes its service again and delivers "fourth" through Tor.
18. A queues "fifth" while B is away and blocks B. With B back for two
    minutes, A makes no session, no attempt and no delivery for the
    blocked record; "fifth" stays queued in an ended conversation.
19. A makes a second identity, a contact of B. A message of the second
    identity is delivered over its own session, while the first identity
    has no session with B and its queue stays as it was; B holds the two
    conversations apart. Once that session lingered and ended, both sides
    send at once: each message arrives once, and one session stays.
20. Every runtime node run is checked: loopback and Unix sockets only, no
    DNS, no UDP.

Phase 6, with the `tor_smoke` example of `monolith-desktop`: the desktop
client driven through its window code (the screens, the UI model, the
bridge and the runtime, in headless frames), one process per client. The
networking itself is shown by the steps above; this step shows that the
client's commands and events drive that runtime over Tor.

21. Each client opens a new profile with the Tor configuration the
    desktop makes of its settings: outside Tails and Whonix a dial waits
    until Tor, asked through the control port with SAFECOOKIE, lists the
    SOCKS endpoint (`GETINFO net/listeners/socks`). Each checks Tor as the
    Tor details dialog does, until the check says Tor works, which needs
    that listing, and makes an identity. B makes an invitation and saves
    its card as "Save card" does; A adds it as a file dropped on "Add
    contact" and writes; B sees A's request, accepts it and answers. Each
    shows its own message Delivered and the other's Received, says so in
    a file, waits for the other's, and closes; the runtime stops. Every
    wait is bounded (15 minutes; 2 for the Tor check to say Tor works),
    and a client that times out names what it waited for and fails.

Phase 8, with `dev-runtime` and generated files (no data of anyone's):

22. Two runtime nodes on new vaults, accepted contacts of each other once
    a first message is delivered. A offers a file of 8 MB by its path,
    then changes the file; B receives the offer with its size and name
    and accepts. While it is on its way each sends a chat message, and B
    receives A's before the file is complete. Both record it complete; B
    is offered the name the file came with, refused a save over a file
    that exists, and saves it: the bytes are those A staged, not the file
    as changed, and no temporary file is left.
23. A file of 64 MB: B is killed with SIGKILL once part of it is
    acknowledged, and started again; it goes on from what it held
    durably, its first acknowledgement after the restart above the last
    one before. Then A is killed and started again. The file is received
    once, completed on both sides and saved whole.
25. A file each way at once, of 16 MB and 12 MB, while chat goes both
    ways: each side completes the other's and saves it, byte for byte.
    The step prints the chat latency during the two transfers, the time
    and throughput, and the peak memory and CPU time of each node.
26. Tor A is stopped while a file of 32 MB is on its way and started
    again 20 seconds later; A publishes again and the file goes on to
    the end.
27. Both nodes start again with their SOCKS endpoint behind a proxy of
    the test that cuts every connection after a number of bytes drawn
    between 256 KiB and 3 MiB, wherever that falls in a frame, and holds
    some of what it forwards back by up to three seconds (seeds 1 and 2,
    each cut logged). A file of 16 MB and a chat message go through the
    cuts, lost sessions and late data; the file arrives whole and once.
28. B is killed as it reports a file complete, maybe before A has its
    acknowledgement; started again, it answers A's next offer from its
    record, A completes, and there is one file, saved once.
29. B deletes A while a file comes, and adds A's card again: nothing of
    the old record stays staged; the new record is offered the file as a
    new offer, from its start, and saves it whole.
30. B blocks A while a file comes: the transfer is cancelled, no chunk
    is taken for two minutes after, and nothing stays staged.
31. A peer built from the tag `phase-7-complete` (`MONOLITH_PHASE7_BIN`):
    A and it become contacts and chat; A's offer of a file is rejected
    once, with no violation reported by the peer; chat goes both ways
    after it, and there is one session at most.
32. `tests/tor_network.rs` in `monolith-core`, on both client Tors: B
    begins a rotation of its transport key and A learns the successor; B
    accepts a file of 3 MB and switches to the new key while it comes;
    the file goes on over a session of the new key, to the same contact
    record, and arrives whole. B deletes A and adds A again: the new
    record holds none of the old one's transfers, also after a session.
24. Every file node run is checked: loopback and Unix sockets only, no
    DNS, no UDP.

Steps 22 and 25 to 31 print the SHA-256 of each file sent and saved.

And again Phase 3, as the last step because it ends Tor B (it was step 3
of Phase 3, hence the gap in the numbers):

12. Tor B goes away while B waits for a stream: B reports the control
    connection lost and fails.

Every Monolith process runs under `strace`, and the trace is checked as in
`tests/network/fail-closed.sh`: every `connect` goes to a loopback address
or a Unix socket, no UDP socket is created, and nothing names port 53. So
there is no direct connection and no DNS. When a step fails, the script
prints what every node said and the state of every node process.

`monolith-two-clients` is the Chutney network definition: three
authorities, five relays and two clients. No public relay, HSDir or other
part of the public network is used, and no Internet access is needed.

Run in the container of `Dockerfile` (Debian 12, Tor from the Tor
Project's repository, Chutney at commit `ae3a33c`), with Monolith built
on the host:

    cargo build --locked -p monolith-cli
    cargo test --locked -p monolith-core --test tor_network --no-run
    cargo build --release --locked -p monolith-desktop --example tor_smoke
    docker build -t monolith-tor-network tests/tor-network
    docker run --rm --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
        -v "$PWD:/src:ro" -v "$PWD/target:/target:ro" \
        -e MONOLITH_BIN=/target/debug/monolith \
        -e MONOLITH_LIFECYCLE_BIN=/target/debug/deps/tor_network-<hash> \
        -e MONOLITH_DESKTOP_SMOKE_BIN=/target/release/examples/tor_smoke \
        monolith-tor-network /src/tests/tor-network/two-node.sh

or directly, with `tor` and `tor-gencert` 0.4.9.5 or later, `strace`,
`cargo` and `python3` on `PATH`:

    git clone https://gitlab.torproject.org/tpo/core/chutney.git
    python3 -m venv chutney-venv && chutney-venv/bin/pip install ./chutney
    PATH=$PWD/chutney-venv/bin:$PATH CHUTNEY_PATH=$PWD/chutney \
        tests/tor-network/two-node.sh

The `tor_smoke` example opens no window: it needs no display and links no
graphics library, only the C library.

The script uses the Chutney interface of 2026 (`init
--net-from-script-path`, and the torrc Chutney writes itself, with a
control socket and cookie authentication in each node's directory).

The script starts and stops Tor processes: test tooling may, Monolith
never does. Tor 0.4.9.13 with `Sandbox 1`, which Chutney sets, can run
with signals blocked in its main thread, SIGTERM among them, depending on
the run; the script kills a Tor that has not ended 10 seconds after
SIGTERM. The test is not part of CI, because it needs Tor and takes about
twenty minutes, and some more with the steps of Phase 5.

Status: steps 1 to 21 passed in the final verification of Phase 7, on
`740b671`, in 1073 seconds, with Tor 0.4.9.13 and Chutney at commit
`ae3a33c` in the container and 3 restart cycles, the default
(`docs/DESIGN_QUESTIONS.md` 14.3), and in that of Phase 6, on
`68ac655`, in 790 seconds (13.3). Steps 1 to 20 had passed in that of
Phase 5, on `91bf22c`, in 2085 seconds with 15 restart cycles (12.8),
and steps 1 to 12 in that of Phase 4, on `970316f` (11.6). The clients
of step 21 also run on the in-memory Tor with `cargo run --release -p
monolith-desktop --example tor_smoke`. The Dockerfile does not pin the
version of Tor. The result that counts for a phase is the run on
the commit of its final verification, recorded in the final report.
