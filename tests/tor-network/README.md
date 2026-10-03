# Private Tor network test

`two-node.sh` is the end-to-end test of Phase 3 on a private Tor network
built with Chutney, the Tor Project's tool for local test networks:

    Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B

1. B publishes a v3 Onion Service; so does A, so that its card names an
   endpoint. Each receives the other's contact card through a file.
2. A connects through its SOCKS port; the Noise XK handshake
   authenticates B to A, then A to B. A sends "hello", B receives it and
   replies "hello back", A receives the reply. Both close the session and
   remove their service with `DEL_ONION`.
3. A service whose control connection went away is published again from
   the key held in memory: it has the same name and is reached again. A
   dial without its SOCKS endpoint fails (`tests/tor_network.rs` in
   `monolith-core`, ignored unless this script sets the endpoints).
4. Tor B goes away while B waits for a stream: B reports the control
   connection lost and fails.

Every Monolith process runs under `strace`, and the trace is checked as in
`tests/network/fail-closed.sh`: every `connect` goes to a loopback address
or a Unix socket, no UDP socket is created, and nothing names port 53. So
there is no direct connection and no DNS.

`monolith-two-clients` is the Chutney network definition: three
authorities, five relays and two clients. No public relay, HSDir or other
part of the public network is used, and no Internet access is needed.

Run:

    git clone https://gitlab.torproject.org/tpo/core/chutney.git
    python3 -m venv chutney-venv && chutney-venv/bin/pip install ./chutney
    PATH=$PWD/chutney-venv/bin:$PATH CHUTNEY_PATH=$PWD/chutney \
        tests/tor-network/two-node.sh

with `tor` and `tor-gencert` 0.4.9.5 or later, `strace`, `cargo` and
`python3` on `PATH`. The script uses the Chutney interface of 2026
(`init --net-from-script-path`, and the torrc Chutney writes itself, with
a control socket and cookie authentication in each node's directory).

The script starts Tor processes: test tooling may, Monolith never does.
It is not part of CI, because it needs Tor and takes minutes.

Status: passed in the final verification of Phase 3, on `2f27dcb`, with
Tor 0.4.9.13 and Chutney at commit `ae3a33c` in a Linux container
(`docs/DESIGN_QUESTIONS.md` 10.4).
