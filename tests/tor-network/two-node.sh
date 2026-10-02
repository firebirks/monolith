#!/bin/bash
# The Phase 3 end-to-end test on a private Tor network, built with Chutney:
#
#   Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B
#
# Both nodes publish an ephemeral Onion Service, exchange their contact
# cards through files, A dials B through its SOCKS port, the handshake
# authenticates both, one message goes each way, and both remove their
# service. No public relay, no public HSDir and no Internet access are
# used. Test tooling may start Tor; Monolith itself never does.
#
# Needs: CHUTNEY_PATH pointing at a Chutney checkout, tor and tor-gencert
# on PATH (0.4.9.5 or later), cargo, python3.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
: "${CHUTNEY_PATH:?set CHUTNEY_PATH to a Chutney checkout}"
network="$root/tests/tor-network/monolith-two-clients"
work=$(mktemp -d)
net="$work/net"
export CHUTNEY_DATA_DIR="$net"

cleanup() {
    "$CHUTNEY_PATH/chutney" stop "$network" >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

cargo build --quiet --locked -p monolith-cli --manifest-path "$root/Cargo.toml"
monolith="$root/target/debug/monolith"

"$CHUTNEY_PATH/chutney" configure "$network"
"$CHUTNEY_PATH/chutney" start "$network"
"$CHUTNEY_PATH/chutney" wait_for_bootstrap "$network"

# The two client nodes, their SOCKS ports and control sockets.
mapfile -t clients < <(ls -d "$net"/nodes/*c | sort)
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
for client in "${clients[@]}"; do
    grep -q '^CookieAuthentication 1' "$client/torrc"
    test -S "$client/control"
done

a="${clients[0]}"
b="${clients[1]}"
"$monolith" --socks "$(endpoint_of "$b")" --control "$(control_of "$b")" tor status

"$monolith" --socks "$(endpoint_of "$b")" --control "$(control_of "$b")" \
    dev-chat serve "$work/b.card" "$work/a.card" >"$work/b.out" 2>&1 &
serve=$!
"$monolith" --socks "$(endpoint_of "$a")" --control "$(control_of "$a")" \
    dev-chat dial "$work/a.card" "$work/b.card" "hello through a private Tor network" \
    >"$work/a.out" 2>&1
wait "$serve"

cat "$work/a.out" "$work/b.out"
grep -q 'Reply: "pong"' "$work/a.out"
grep -q 'Received: "hello through a private Tor network"' "$work/b.out"
grep -q 'Done; service removed.' "$work/a.out"
grep -q 'Done; service removed.' "$work/b.out"
echo "Two-node test passed."
