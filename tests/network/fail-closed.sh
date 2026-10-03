#!/bin/bash
# T-NET-1 on a development machine: Monolith's Tor client paths make no
# network system call other than to the configured loopback endpoints, use
# no DNS, and fail when Tor is not there. See README.md in this directory.
#
# Needs: cargo, python3, strace, and unprivileged network namespaces
# (unshare -rn). Runs no Tor and uses no network.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cargo build --quiet --locked -p monolith-cli
monolith="${CARGO_TARGET_DIR:-$root/target}/debug/monolith"
tests=$(cargo test --quiet --locked -p monolith-tor --test system --no-run --message-format=json 2>/dev/null |
    python3 -c '
import json, sys
for line in sys.stdin:
    message = json.loads(line)
    if message.get("target", {}).get("name") == "system" and message.get("executable"):
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
print("ok   " + label)
PY
}

# Runs a command under strace, checks its network calls, and returns the
# command's own exit status.
traced() {
    local label=$1 status=0
    shift
    strace -f -qq -e trace=socket,connect,sendto,sendmsg -o "$work/$label.log" "$@" || status=$?
    # A failed check ends the script, whatever context the call is in.
    check "$work/$label.log" "$label" >&2 || exit 1
    return "$status"
}

closed_port() {
    python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1]); s.close()'
}

echo "1. SOCKS available: the onion name goes to the proxy, nothing else is contacted."
traced socks-available "$tests" --exact --quiet \
    an_onion_connection_carries_the_literal_hostname_and_then_the_peer_bytes

echo "2. SOCKS unavailable, direct Internet available: the dial fails, nothing else is tried."
traced socks-unavailable "$tests" --exact --quiet \
    without_a_socks_endpoint_there_is_no_connection_at_all

echo "3. The CLI with Tor absent reports offline and contacts only its endpoints."
status=0
traced cli-tor-absent "$monolith" --socks "127.0.0.1:$(closed_port)" \
    --control "127.0.0.1:$(closed_port)" tor status >"$work/cli.out" || status=$?
grep -q "offline: Tor is unavailable" "$work/cli.out"
test "$status" = 1
echo "ok   cli reports offline, exit 1"

echo "4. A network namespace with loopback only: no DNS exists, onion connections still work."
unshare -rn bash -c '
    set -eu
    ip link set lo up
    strace -f -qq -e trace=socket,connect,sendto,sendmsg -o "$1/netns.log" \
        "$2" --exact --quiet \
        an_onion_connection_carries_the_literal_hostname_and_then_the_peer_bytes \
        without_a_socks_endpoint_there_is_no_connection_at_all
' _ "$work" "$tests"
check "$work/netns.log" netns-loopback-only || exit 1

echo "T-NET-1 passed."
