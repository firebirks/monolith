# Private Tor network test

`two-node.sh` is the end-to-end test of Phase 3 on a private Tor network
built with Chutney, the Tor Project's tool for local test networks:

    Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B

1. B publishes a v3 Onion Service; so does A, so that its card names an
   endpoint.
2. Each receives the other's contact card through a file.
3. A connects through its SOCKS port.
4. The Noise XK handshake authenticates B to A, then A to B.
5. A sends a message; B receives it and replies; A receives the reply.
6. Both close the session and remove their service with `DEL_ONION`.

`monolith-two-clients` is the Chutney network definition: three
authorities, five relays and two clients. No public relay, HSDir or other
part of the public network is used, and no Internet access is needed.

Run:

    git clone https://gitlab.torproject.org/tpo/core/chutney.git
    CHUTNEY_PATH=$PWD/chutney tests/tor-network/two-node.sh

with `tor` and `tor-gencert` 0.4.9.5 or later on `PATH`.

The script starts Tor processes: test tooling may, Monolith never does.
It is not part of CI, because it needs Tor and takes minutes.

Status: written but not yet run. The development environment of Phase 3
has no Tor; the run is open (see `docs/TEST_PLAN.md`). The Chutney names
it relies on (the network file format, `wait_for_bootstrap`, the
`*c` client node directories with `torrc` and `control`) should be
checked against the Chutney version used on the first run.
