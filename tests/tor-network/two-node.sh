#!/bin/bash
# The Phase 3 end-to-end test on a private Tor network, built with Chutney:
#
#   Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B
#
# 1. Both nodes publish an ephemeral Onion Service and exchange their
#    contact cards through files; A dials B through its SOCKS port, the
#    handshake authenticates both, "hello" goes to B and "hello back" to A,
#    and both remove their service.
# 2. A service whose control connection went away is published again from
#    the key held in memory, under the same name, and reached again; a dial
#    without its SOCKS endpoint fails.
# 3. Tor B goes away while B waits for a stream: B reports the control
#    connection lost and does not claim the service is still there.
#
# Every Monolith process runs under strace, and every connect it makes
# must go to a loopback address or a Unix socket, with no UDP socket and
# nothing on port 53: no direct connection and no DNS. No public relay,
# no public HSDir and no Internet access are used. Test tooling may start
# Tor; Monolith itself never does.
#
# Needs: CHUTNEY_PATH pointing at a Chutney checkout, tor and tor-gencert
# on PATH (0.4.9.5 or later), cargo, python3, strace.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
: "${CHUTNEY_PATH:?set CHUTNEY_PATH to a Chutney checkout}"
network="$root/tests/tor-network/monolith-two-clients"
work=$(mktemp -d)
net="$work/net"
chutney() { "$CHUTNEY_PATH/chutney" --data-dir "$net" "$@"; }

cleanup() {
    chutney stop >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

cargo build --quiet --locked -p monolith-cli --manifest-path "$root/Cargo.toml"
monolith="${CARGO_TARGET_DIR:-$root/target}/debug/monolith"
lifecycle=$(cargo test --quiet --locked -p monolith-core --test tor_network --no-run \
    --message-format=json --manifest-path "$root/Cargo.toml" 2>/dev/null |
    python3 -c '
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("target", {}).get("name") == "tor_network" and message.get("executable"):
        print(message["executable"])' | tail -1)

# Every connect must go to 127.0.0.0/8, ::1 or a Unix socket; no UDP
# socket may be created (DNS is UDP); nothing may name port 53.
check() {
    python3 - "$1" "$2" <<'PY'
import re, sys
log, label = sys.argv[1], sys.argv[2]
bad = []
for line in open(log):
    if re.search(r"socket\((AF_INET6?|AF_PACKET|AF_NETLINK)[^)]*SOCK_DGRAM", line):
        bad.append(line.strip())
    if "AF_PACKET" in line or "htons(53)" in line:
        bad.append(line.strip())
    match = re.search(r"connect\(\d+, \{sa_family=(AF_\w+)(.*?)\}", line)
    if match:
        family, rest = match.groups()
        if family == "AF_UNIX":
            continue
        if family == "AF_INET" and 'inet_addr("127.' in rest:
            continue
        if family == "AF_INET6" and '"::1"' in rest:
            continue
        bad.append(line.strip())
if bad:
    print("FAIL " + label)
    for line in bad:
        print("     " + line)
    sys.exit(1)
print("ok   " + label + ": loopback and Unix sockets only, no DNS")
PY
}

# Runs a command under strace, checks its network calls, and returns the
# command's own exit status.
traced() {
    local label=$1 status=0
    shift
    strace -f -qq -e trace=socket,connect,sendto,sendmsg -o "$work/$label.log" "$@" || status=$?
    check "$work/$label.log" "$label" >&2 || exit 1
    return "$status"
}

closed_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

chutney init --net-from-script-path "$network"
chutney configure
chutney start
chutney wait_for_bootstrap

# The two client nodes, their SOCKS ports, control sockets and cookies.
mapfile -t clients < <(find "$net" -maxdepth 2 -type d -name '[0-9][0-9][0-9]c' | sort)
test "${#clients[@]}" = 2
socks_of() { awk '/^SocksPort/ { print $2; exit }' "$1/torrc"; }
endpoint_of() {
    local port
    port=$(socks_of "$1")
    case "$port" in
        *:*) echo "$port" ;;
        *) echo "127.0.0.1:$port" ;;
    esac
}
control_of() { echo "unix:$1/control"; }
cookie_of() { echo "$1/control_auth_cookie"; }
for client in "${clients[@]}"; do
    grep -q '^CookieAuthentication 1' "$client/torrc"
    test -S "$client/control"
done
a="${clients[0]}"
b="${clients[1]}"
tor_a=(--socks "$(endpoint_of "$a")" --control "$(control_of "$a")" --cookie-file "$(cookie_of "$a")")
tor_b=(--socks "$(endpoint_of "$b")" --control "$(control_of "$b")" --cookie-file "$(cookie_of "$b")")

tor --version | head -1
# Wait until both client Tors have a circuit and report themselves ready.
for side in a b; do
    if [ "$side" = a ]; then opts=("${tor_a[@]}"); else opts=("${tor_b[@]}"); fi
    for _ in $(seq 60); do
        "$monolith" "${opts[@]}" tor status >"$work/status-$side.out" 2>&1 && break
        sleep 2
    done
    cat "$work/status-$side.out"
    "$monolith" "${opts[@]}" tor status >/dev/null
done

echo "1. A dials B through the private network; one message each way."
traced serve "$monolith" "${tor_b[@]}" dev-chat serve "$work/b.card" "$work/a.card" \
    >"$work/b.out" 2>&1 &
serve=$!
traced dial "$monolith" "${tor_a[@]}" dev-chat dial "$work/a.card" "$work/b.card" "hello" \
    >"$work/a.out" 2>&1
wait "$serve"
cat "$work/a.out" "$work/b.out"
grep -q 'Received: "hello"$' "$work/b.out"
grep -q 'Reply: "hello back"$' "$work/a.out"
grep -q 'Done; service removed.' "$work/a.out"
grep -q 'Done; service removed.' "$work/b.out"
echo "ok   B received \"hello\", A received \"hello back\""

echo "2. Published again from the key in memory; a dial without SOCKS fails."
MONOLITH_TOR_A_SOCKS=$(endpoint_of "$a") MONOLITH_TOR_A_CONTROL=$(control_of "$a") \
    MONOLITH_TOR_A_COOKIE=$(cookie_of "$a") \
    MONOLITH_TOR_B_SOCKS=$(endpoint_of "$b") MONOLITH_TOR_B_CONTROL=$(control_of "$b") \
    MONOLITH_TOR_B_COOKIE=$(cookie_of "$b") \
    MONOLITH_TOR_CLOSED_SOCKS="127.0.0.1:$(closed_port)" \
    traced lifecycle "$lifecycle" --ignored --exact --nocapture \
    a_service_published_again_from_its_key_is_reached_again >"$work/lifecycle.out" 2>&1 || {
    cat "$work/lifecycle.out"
    exit 1
}
cat "$work/lifecycle.out"
grep -q 'Published again under the same name.' "$work/lifecycle.out"
test "$(grep -c 'Alice received "hello back"' "$work/lifecycle.out")" = 2
grep -q 'Without SOCKS the dial failed.' "$work/lifecycle.out"
echo "ok   same name after publishing again, reached twice; no SOCKS, no connection"

echo "3. Tor B goes away while B waits: the control connection is reported lost."
status=0
# B has its peer's card at once, so that it is waiting for a stream when
# its Tor goes away.
cp "$work/a.card" "$work/a2.card"
traced control-lost "$monolith" "${tor_b[@]}" dev-chat serve "$work/b2.card" "$work/a2.card" \
    >"$work/b2.out" 2>&1 &
serve=$!
for _ in $(seq 120); do
    test -s "$work/b2.card" && break
    sleep 1
done
test -s "$work/b2.card"
sleep 2
kill "$(cat "$b/pid")"
wait "$serve" || status=$?
cat "$work/b2.out"
test "$status" != 0
grep -q 'Tor control connection lost' "$work/b2.out"
echo "ok   B failed with the control connection lost"

echo "Two-node test passed."
