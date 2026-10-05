# Private Tor network test

`two-node.sh` is the end-to-end test of Phases 3 and 4 on a private Tor
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
9. Tor B is restarted while a session is open: B reports both services
   unavailable, its supervisor publishes them again under the same names,
   and A completes Noise XK with B again.
10. A without its SOCKS endpoint, and A without any Tor while the machine
    has direct Internet access: every dial fails at once, and nothing is
    contacted.
11. Every node run is checked: loopback and Unix sockets only, no DNS.

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
    docker build -t monolith-tor-network tests/tor-network
    docker run --rm --cap-add SYS_PTRACE --security-opt seccomp=unconfined \
        -v "$PWD:/src:ro" -v "$PWD/target:/target:ro" \
        -e MONOLITH_BIN=/target/debug/monolith \
        -e MONOLITH_LIFECYCLE_BIN=/target/debug/deps/tor_network-<hash> \
        monolith-tor-network /src/tests/tor-network/two-node.sh

or directly, with `tor` and `tor-gencert` 0.4.9.5 or later, `strace`,
`cargo` and `python3` on `PATH`:

    git clone https://gitlab.torproject.org/tpo/core/chutney.git
    python3 -m venv chutney-venv && chutney-venv/bin/pip install ./chutney
    PATH=$PWD/chutney-venv/bin:$PATH CHUTNEY_PATH=$PWD/chutney \
        tests/tor-network/two-node.sh

The script uses the Chutney interface of 2026 (`init
--net-from-script-path`, and the torrc Chutney writes itself, with a
control socket and cookie authentication in each node's directory).

The script starts and stops Tor processes: test tooling may, Monolith
never does. Tor 0.4.9.13 with `Sandbox 1`, which Chutney sets, can run
with signals blocked in its main thread, SIGTERM among them, depending on
the run; the script kills a Tor that has not ended 10 seconds after
SIGTERM. The test is not part of CI, because it needs Tor and takes about
twenty minutes.

Status: runs with Tor 0.4.9.13 and Chutney at commit `ae3a33c` in the
container. The result that counts for a phase is the run on the commit of
its final verification, recorded in the final report.
