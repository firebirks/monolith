# Network tests

## fail-closed.sh (T-NET-1)

Shows on a development machine that Monolith's Tor client paths contact
nothing but the configured Tor endpoints, resolve no names, and fail when
Tor is not there:

1. SOCKS available: an onion connection sends the `.onion` name to the
   proxy as a SOCKS domain name and contacts nothing else.
2. SOCKS unavailable while the machine has direct Internet access: the
   dial fails, and nothing else is tried.
3. `monolith tor status` with no Tor: it reports offline, exits 1, and
   contacts only its two configured loopback endpoints.
4. In a network namespace with only loopback, where no DNS and no route
   exist, the onion connection logic works as before.

Every case runs under `strace`, and the trace is checked: every
`connect` goes to 127.0.0.0/8, `::1` or a Unix socket, no UDP socket is
created (DNS is UDP), and nothing names port 53. A system call trace sees
attempts as well as traffic, which a packet capture would not.

    tests/network/fail-closed.sh

Needs `cargo`, `python3`, `strace`, and unprivileged network namespaces
(`unshare -rn`). It starts no Tor and needs no network.

What it does not show: what happens inside Tor, and the behavior of a
real system Tor, which the private network test covers. On Tails the
property cannot be shown by blocking, because Tails redirects direct TCP
into Tor; see `docs/TEST_PLAN.md` section 11.
