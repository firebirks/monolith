#!/bin/bash
# The end-to-end test on a private Tor network, built with Chutney:
#
#   Monolith A -- Tor A -- private Tor network -- Tor B -- Monolith B
#
# Phase 3:
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
# Phase 4, with `dev-node`, a node over a persistent vault driven through
# its standard input (steps 4 to 11 below say what each shows): contacts
# and successors survive a restart of the node; key retirement, blocking
# and deletion end live sessions; invitations decide requests that arrive
# over Tor; one node holds two local identities; a restart of the Tor
# process is followed by the supervisor publishing the service again under
# the same name and a new Noise XK exchange; without SOCKS, or without
# Tor at all while direct Internet access exists, nothing is contacted.
#
# Phase 5, with `dev-runtime`, a node on the messaging runtime (steps 13
# to 20, run before step 12, which ends Tor B): a message queued while the
# peer is away is delivered once, stored and acknowledged; a crash of the
# receiver between storing and acknowledging, a crash of the sender with a
# queued message and a restart of the sender's Tor are followed by
# delivery, without a second copy; a block stops a queue; two identities of
# one node keep their queues and sessions apart; both sides sending at once
# end with one session.
#
# Every Monolith process runs under strace, and every connect it makes
# must go to a loopback address or a Unix socket, with no UDP socket and
# nothing on port 53: no direct connection and no DNS. No public relay,
# no public HSDir and no Internet access are used. Test tooling may start
# Tor; Monolith itself never does.
#
# Needs: CHUTNEY_PATH pointing at a Chutney checkout, tor and tor-gencert
# on PATH (0.4.9.5 or later), python3, strace, and either cargo or the
# binaries prebuilt: MONOLITH_BIN for `monolith` and MONOLITH_LIFECYCLE_BIN
# for the `tor_network` test of monolith-core (the Dockerfile here runs it
# that way).
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
: "${CHUTNEY_PATH:?set CHUTNEY_PATH to a Chutney checkout}"
network="$root/tests/tor-network/monolith-two-clients"
work=$(mktemp -d)
net="$work/net"
chutney() { "$CHUTNEY_PATH/chutney" --data-dir "$net" "$@"; }

cleanup() {
    local status=$?
    if [ "$status" != 0 ]; then
        # What every Monolith process said, for the failure report.
        for out in "$work"/*.out; do
            [ -e "$out" ] || continue
            echo "--- $(basename "$out")"
            tail -30 "$out"
        done
    fi
    chutney stop >/dev/null 2>&1 || true
    rm -rf "$work"
}
trap cleanup EXIT

if [ -n "${MONOLITH_BIN:-}" ]; then
    monolith=$MONOLITH_BIN
    lifecycle=${MONOLITH_LIFECYCLE_BIN:?set MONOLITH_LIFECYCLE_BIN with MONOLITH_BIN}
else
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
fi

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

# Ends the Tor process recorded in `$1/pid` and waits until it is gone.
# Tor 0.4.9.13 with `Sandbox 1`, which Chutney sets, can run with signals
# blocked in its main thread, SIGTERM among them, depending on the run; a
# Tor that has not ended 10 seconds after SIGTERM is killed.
stop_tor() {
    local pid i
    pid=$(cat "$1/pid")
    kill "$pid" 2>/dev/null || true
    for ((i = 0; i < 20; i++)); do
        if [ ! -e "/proc/$pid" ] || [ "$(awk '{ print $3 }' "/proc/$pid/stat")" = Z ]; then
            return 0
        fi
        if [ "$i" = 10 ]; then
            echo "     (Tor did not end on SIGTERM; killed)"
            kill -KILL "$pid" 2>/dev/null || true
        fi
        sleep 1
    done
    echo "FAIL Tor $pid did not end"
    exit 1
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

# --- Phase 4: nodes over a persistent vault ---------------------------------

printf 'test passphrase\n' >"$work/pass"
declare -A pids fds
traces=0

# Starts node `name` on the vault in `data` with the Tor options given,
# under strace, reading commands from a FIFO that stays open.
start_node() {
    local name=$1 data=$2 fd
    shift 2
    rm -f "$work/$name.in"
    mkfifo "$work/$name.in"
    touch "$work/$name.out"
    traces=$((traces + 1))
    # Reads are traced with their first two bytes only, enough for the
    # reply code of the SOCKS endpoint (step 9).
    strace -f -qq -s 2 -x -e trace=socket,connect,sendto,sendmsg,read,recvfrom \
        -o "$work/trace-$name-$traces.log" \
        "$monolith" "$@" dev-node "$data" "$work/pass" --kdf-floor --echo \
        <"$work/$name.in" >>"$work/$name.out" 2>&1 &
    pids[$name]=$!
    exec {fd}>"$work/$name.in"
    fds[$name]=$fd
}

# Sends a command line to node `name`.
tell() {
    local name=$1
    shift
    echo "$*" >&"${fds[$name]}"
}

# Waits until the output of `name` holds at least `n` lines matching
# `pattern`, for up to `timeout` seconds.
expect() {
    local name=$1 pattern=$2 n=$3 timeout=$4 i
    for ((i = 0; i < timeout; i++)); do
        if [ "$(grep -cE -- "$pattern" "$work/$name.out" || true)" -ge "$n" ]; then
            return 0
        fi
        sleep 1
    done
    echo "FAIL $name: fewer than $n lines matching '$pattern' after ${timeout}s"
    # What every node process was doing: state, CPU time and wait channel
    # of each thread, twice, two seconds apart.
    for p in $(pgrep -f " dev-(node|runtime) " || true); do
        for round in 1 2; do
            for t in /proc/"$p"/task/*; do
                echo "     pid $p thread ${t##*/}: $(cut -d' ' -f3,14,15 "$t/stat") $(cat "$t/wchan")"
            done
            [ "$round" = 2 ] || sleep 2
        done
    done
    tail -40 "$work/$name.out"
    exit 1
}

# The number of lines of `name` matching `pattern`.
count() {
    grep -cE -- "$2" "$work/$1.out" || true
}

# The last line of `name` matching `pattern`.
last() {
    grep -E -- "$2" "$work/$1.out" | tail -1
}

# Stops node `name` and checks the network calls of all its runs so far.
stop_node() {
    local name=$1 log
    tell "$name" quit
    wait "${pids[$name]}"
    exec {fds[$name]}>&-
    for log in "$work"/trace-"$name"-*.log; do
        check "$log" "$name $(basename "$log" .log)" || exit 1
    done
}

data_a="$work/node-a"
data_b="$work/node-b"

echo "4. Two nodes over vaults: each makes an identity, imports the other's card, hello."
start_node a "$data_a" "${tor_a[@]}"
start_node b "$data_b" "${tor_b[@]}"
expect a '^created$' 1 60
expect b '^created$' 1 60
tell a new-identity
tell b new-identity
expect a '^published 0$' 1 120
expect b '^published 0$' 1 120
id_a=$(last a '^identity 0 ' | cut -d' ' -f3)
id_b=$(last b '^identity 0 ' | cut -d' ' -f3)
tell a card 0
tell b card 0
expect a '^card 0 ' 1 10
expect b '^card 0 ' 1 10
card_a=$(last a '^card 0 ' | cut -d' ' -f3)
card_b=$(last b '^card 0 ' | cut -d' ' -f3)
tell a add 0 "$card_b"
tell b add 0 "$card_a"
expect a "^added 0 $id_b created$" 1 10
expect b "^added 0 $id_a created$" 1 10
tell a send 0 "$id_b" hello
expect b "^message 0 $id_a \"hello\"$" 1 400
expect a "^message 0 $id_b \"hello back\"$" 1 60
expect a "^peer-accepted 0 $id_b$" 1 10
expect b "^peer-accepted 0 $id_a$" 1 10
echo "ok   both accepted each other through the protocol; hello and hello back"

echo "5. B restarts: its contact, its identity and its service are as before."
stop_node b
start_node b "$data_b" "${tor_b[@]}"
expect b '^opened Clean$' 1 60
expect b "^identity 0 $id_b$" 2 10
expect b '^published 0$' 2 120
tell b card 0
expect b '^card 0 ' 1 10
test "$(last b '^card 0 ' | cut -d' ' -f3)" = "$card_b"
tell b show 0 "$id_a"
expect b "^contact 0 $id_a kind=Accepted active=1 " 1 10
expect a "^ended 0 $id_b " 1 60
tell a send 0 "$id_b" hello
expect a "^outbound 0 $id_b Accepted$" 1 400
expect b "^inbound 0 $id_a Accepted$" 1 60
expect a "^message 0 $id_b \"hello back\"$" 2 60
echo "ok   B kept A as an accepted contact across its restart; same card, same service"

echo "6. A rotates its key: B holds the successor across a restart, then promotes it."
tell a rotate 0 begin
expect a '^rotation 0 begin done=true state=announcing$' 1 10
# A session confirmed after the start of the rotation carries the successor.
tell a close 0 "$id_b"
expect a "^ended 0 $id_b closed$" 1 30
tell a send 0 "$id_b" hello
expect a "^announced 0 $id_b$" 1 400
expect b "^endpoint-update 0 $id_a Authorized$" 1 60
tell b show 0 "$id_a"
expect b "^contact 0 $id_a kind=Accepted active=1 authorized=2 " 1 10
stop_node b
start_node b "$data_b" "${tor_b[@]}"
expect b '^opened Clean$' 2 60
expect b '^published 0$' 3 120
tell b show 0 "$id_a"
expect b "^contact 0 $id_a kind=Accepted active=1 authorized=2 " 2 10
echo "ok   the authorized successor survived B's restart"
# A session of the old key stays open while A dials with the new key.
expect a "^ended 0 $id_b " 3 60
tell a send 0 "$id_b" hello
expect a "^confirmed 0 $id_b key=old$" 4 400
tell a rotate 0 switch
expect a '^rotation 0 switch done=true state=switched$' 1 10
tell a dial 0 "$id_b" hello
expect a "^confirmed 0 $id_b key=new$" 1 400
expect a "^promoted-successor 0 $id_b$" 1 60
# B promoted the new key and withdrew the session of the old one.
expect b "^ended 0 $id_a withdrawn$" 1 60
tell b show 0 "$id_a"
expect b "^contact 0 $id_a kind=Accepted active=2 authorized=0 pending=0 retired=true " 1 10
tell a rotate 0 finish
expect a '^rotation 0 finish done=true state=none$' 1 10
stop_node b
start_node b "$data_b" "${tor_b[@]}"
expect b '^published 0$' 4 120
tell b show 0 "$id_a"
expect b "^contact 0 $id_a kind=Accepted active=2 authorized=0 pending=0 retired=true " 2 10
echo "ok   promoted at B, old session withdrawn, retired key kept across a restart"

echo "7. B blocks A while their session is open: it ends, and A is a stranger to B."
expect a "^ended 0 $id_b " 5 60
tell a send 0 "$id_b" hello
expect b "^inbound 0 $id_a Accepted$" 5 400
expect a "^message 0 $id_b \"hello back\"$" 6 60
tell b block 0 "$id_a"
expect b "^blocked 0 $id_a$" 1 10
expect b "^ended 0 $id_a withdrawn$" 2 30
expect a "^ended 0 $id_b " 6 60
hellos=$(count a "^message 0 $id_b \"hello back\"$")
tell a send 0 "$id_b" hello
expect b "^inbound 0 $id_a Blocked$" 1 400
expect a "^ended 0 $id_b " 7 120
test "$(count a "^message 0 $id_b \"hello back\"$")" = "$hellos"
tell b unblock 0 "$id_a"
expect b "^unblocked 0 $id_a$" 1 10
echo "ok   the block withdrew the open session; a blocked peer gets no reply"

echo "8. A second identity at B: invitation mode, a capability, deletion ends the session."
tell b new-identity
expect b '^identity 1 ' 1 120
expect b '^published 1$' 1 120
id_b1=$(last b '^identity 1 ' | cut -d' ' -f3)
tell b card 1
expect b '^card 1 ' 1 10
open_b1=$(last b '^card 1 ' | cut -d' ' -f3)
tell a add 0 "$open_b1"
expect a "^added 0 $id_b1 created$" 1 10
tell a dial 0 "$id_b1"
expect b "^request 1 $id_a dropped:Mode$" 1 400
tell b invite 1 Website
expect b '^invitation 1 ' 1 10
invitation=$(last b '^invitation 1 ' | cut -d' ' -f3)
invited_b1=$(last b '^invitation 1 ' | cut -d' ' -f4)
tell a add 0 "$invited_b1"
expect a "^added 0 $id_b1 Unchanged$" 1 10
tell a dial 0 "$id_b1"
expect b "^request 1 $id_a queued$" 1 400
tell b requests 1
expect b '^requests 1 1$' 1 10
tell b accept 1 "$id_a"
expect b "^accepted 1 $id_a$" 1 10
tell a dial 0 "$id_b1" hello
expect b "^message 1 $id_a \"hello\"$" 1 400
expect a "^message 0 $id_b1 \"hello back\"$" 1 60
# The two identities of B hold A differently: none at 0, accepted at 1.
tell b show 1 "$id_a"
expect b "^contact 1 $id_a kind=Accepted " 1 10
tell b show 0 "$id_a"
expect b '^error no such peer$' 1 10
# Revoking the capability does not touch the accepted contact.
tell b revoke 1 "$invitation"
expect b '^revoked 1$' 1 10
tell b show 1 "$id_a"
expect b "^contact 1 $id_a kind=Accepted " 2 10
tell b delete 1 "$id_a"
expect b "^deleted 1 $id_a$" 1 10
expect b "^ended 1 $id_a withdrawn$" 1 30
echo "ok   requests decided by mode and capability; identities apart; deletion withdrew the session"

echo "9. Tor B restarts: the supervisor publishes both services again; hello again."
# The reply codes Tor A's SOCKS endpoint gave node A, in order, read from
# the trace of A. On each connection to the endpoint `monolith-tor` reads
# the method reply and the authentication reply, two bytes each, and then
# the CONNECT reply a byte at a time: its code is the fourth read.
socks_replies() {
    python3 - "$1" "$2" <<'PY'
import re, sys
trace, port = sys.argv[1], sys.argv[2]
reads, replies = {}, []
for line in open(trace, errors="replace"):
    connect = re.search(r"connect\((\d+), \{sa_family=AF_INET, sin_port=htons\((\d+)\)", line)
    if connect:
        if connect.group(2) == port:
            reads[connect.group(1)] = 0
        else:
            reads.pop(connect.group(1), None)
        continue
    read = re.search(r"(?:read|recvfrom)\((\d+), \"((?:\\x[0-9a-f]{2})+)\"", line)
    if read and read.group(1) in reads:
        reads[read.group(1)] += 1
        if reads[read.group(1)] == 4:
            replies.append("0x" + read.group(2)[2:4])
print(" ".join(replies))
PY
}
socks_port_a=$(endpoint_of "$a" | sed 's/.*://')
# After a restart of Tor B, A sends to B0 until a session is confirmed. A
# dial refused by Tor A's SOCKS endpoint (Tor may refuse while the service
# comes back) is sent again; any other failure fails the test, and so does
# no session within $recovery_window seconds of the first try. The
# attempts, the time and the SOCKS replies are printed.
# Tor may hold the first stream after the restart for its SocksTimeout,
# 120 seconds, and then refuse it with 0x01; the window allows two such
# waits and no more.
restart_cycles=${RESTART_CYCLES:-3}
recovery_window=${RECOVERY_WINDOW:-240}
recover() {
    local cycle=$1 started elapsed attempts=0 confirmed failed messages retries reason trace seen replies
    local lines_a lines_b
    trace=$(ls -t "$work"/trace-a-*.log | head -1)
    seen=$(socks_replies "$trace" "$socks_port_a" | wc -w)
    retries=$(count a "^dial-retry 0 $id_b ")
    messages=$(count a "^message 0 $id_b \"hello back\"$")
    lines_a=$(wc -l <"$work/a.out")
    lines_b=$(wc -l <"$work/b.out")
    started=$(date +%s)
    while :; do
        attempts=$((attempts + 1))
        confirmed=$(count a "^confirmed 0 $id_b ")
        failed=$(count a "^dial-failed 0 $id_b ")
        tell a send 0 "$id_b" hello
        while [ "$(count a "^confirmed 0 $id_b ")" -eq "$confirmed" ] &&
            [ "$(count a "^dial-failed 0 $id_b ")" -eq "$failed" ]; do
            elapsed=$(($(date +%s) - started))
            if [ "$elapsed" -ge "$recovery_window" ]; then
                replies=$(socks_replies "$trace" "$socks_port_a" | cut -d' ' -f$((seen + 1))-)
                echo "FAIL cycle $cycle: no session within ${recovery_window}s; attempts=$attempts socks replies: ${replies:-none}"
                tail -20 "$work/a.out"
                exit 1
            fi
            sleep 1
        done
        if [ "$(count a "^confirmed 0 $id_b ")" -gt "$confirmed" ]; then
            break
        fi
        reason=$(last a "^dial-failed 0 $id_b " | cut -d' ' -f4-)
        elapsed=$(($(date +%s) - started))
        if [ "$reason" != "Tor SOCKS endpoint refused the request" ] ||
            [ "$elapsed" -ge "$recovery_window" ]; then
            replies=$(socks_replies "$trace" "$socks_port_a" | cut -d' ' -f$((seen + 1))-)
            echo "FAIL cycle $cycle: dial failed: $reason; attempts=$attempts after ${elapsed}s; socks replies: ${replies:-none}"
            exit 1
        fi
        sleep 1
    done
    expect a "^message 0 $id_b \"hello back\"$" $((messages + 1)) 120
    elapsed=$(($(date +%s) - started))
    replies=$(socks_replies "$trace" "$socks_port_a" | cut -d' ' -f$((seen + 1))-)
    echo "     cycle $cycle: attempts=$attempts elapsed=${elapsed}s" \
        "dial-retries=$(($(count a "^dial-retry 0 $id_b ") - retries))" \
        "socks replies: ${replies:-none}"
    # What a slow recovery went through, for the record.
    if [ "$elapsed" -gt 10 ]; then
        tail -n +$((lines_a + 1)) "$work/a.out" | sed 's/^/       a: /'
        tail -n +$((lines_b + 1)) "$work/b.out" | sed 's/^/       b: /'
    fi
}
# A contact of B0 again, and a session between them before the restart.
tell a card 0
expect a '^card 0 ' 2 10
card_a=$(last a '^card 0 ' | cut -d' ' -f3)
tell b add 0 "$card_a"
expect b "^added 0 $id_a created$" 2 10
tell a send 0 "$id_b" hello
expect a "^message 0 $id_b \"hello back\"$" 7 400
for ((cycle = 1; cycle <= restart_cycles; cycle++)); do
    # From the second cycle on, the services ran long enough for the
    # supervisor to reset its delays (RECONNECT_RESET_AFTER, 60 seconds,
    # docs/RESOURCE_LIMITS.md section 7): each restart finds them as a
    # restart after a while does, and is followed by the first delay only.
    if [ "$cycle" -gt 1 ]; then
        sleep 65
    fi
    before_b=$(count b '^published 0$')
    before_b1=$(count b '^published 1$')
    gone_b=$(count b '^unavailable 0 ')
    gone_b1=$(count b '^unavailable 1 ')
    cards_b=$(count b '^card 0 ')
    ended_a=$(count a "^ended 0 $id_b ")
    stop_tor "$b"
    expect b '^unavailable 0 ' $((gone_b + 1)) 60
    expect b '^unavailable 1 ' $((gone_b1 + 1)) 60
    tor -f "$b/torrc" >>"$work/tor-b-restarted.log" 2>&1 &
    tor_b_pid=$!
    # Test tooling restarted it; Chutney stops it, and step 12 ends it, by
    # its pid file.
    echo "$tor_b_pid" >"$b/pid"
    expect b '^published 0$' $((before_b + 1)) 300
    expect b '^published 1$' $((before_b1 + 1)) 300
    # The same card, so the same Onion Service name and key.
    tell b card 0
    expect b '^card 0 ' $((cards_b + 1)) 10
    test "$(last b '^card 0 ' | cut -d' ' -f3)" = "$card_b"
    # A's session ended with the stream, when its Tor learned that the
    # circuit is gone, or at the latest at the idle deadline of the session.
    expect a "^ended 0 $id_b " $((ended_a + 1)) 300
    # A session confirmed is Noise XK with B0's key at that name.
    recover "$cycle"
done
echo "ok   republished under the same name after $restart_cycles restart(s) of Tor B; Noise XK and hello again"

echo "10. Without SOCKS, and without Tor while direct Internet access exists: nothing."
stop_node a
python3 -c 'import sys; sys.exit(0 if any(line.split()[1] == "00000000" for line in open("/proc/net/route").readlines()[1:]) else 1)' &&
    echo "     (this machine has a default route: direct Internet access is there)"
published=$(count a '^published 0$')
start_node a "$data_a" --socks "127.0.0.1:$(closed_port)" --control "${tor_a[3]}" \
    --cookie-file "${tor_a[5]}"
expect a '^published 0$' $((published + 1)) 120
tell a send 0 "$id_b" hello
expect a "^dial-failed 0 $id_b Tor SOCKS endpoint unavailable$" 1 60
stop_node a
unavailable=$(count a '^unavailable 0 ')
start_node a "$data_a" --socks "127.0.0.1:$(closed_port)" --control "127.0.0.1:$(closed_port)" \
    --cookie-file "${tor_a[5]}"
expect a '^unavailable 0 Tor control endpoint unavailable$' $((unavailable + 1)) 60
tell a send 0 "$id_b" hello
expect a "^dial-failed 0 $id_b Tor SOCKS endpoint unavailable$" 2 60
stop_node a
echo "ok   both dials failed at once; the traces show only loopback and Unix sockets"

echo "11. Every node run contacted only loopback and Unix sockets, no DNS."
stop_node b
echo "ok   $traces node runs checked"

# --- Phase 5: nodes on the messaging runtime --------------------------------

# Starts runtime node `name` on the vault in `data`, acknowledging after
# `delay` seconds, with the Tor options given, under strace, reading
# commands from a FIFO that stays open. Returns once the node is ready.
start_runtime() {
    local name=$1 data=$2 delay=$3 fd ready
    shift 3
    rm -f "$work/$name.in"
    mkfifo "$work/$name.in"
    touch "$work/$name.out"
    ready=$(count "$name" '^ready$')
    traces=$((traces + 1))
    strace -f -qq -e trace=socket,connect,sendto,sendmsg \
        -o "$work/trace-$name-$traces.log" \
        "$monolith" "$@" dev-runtime "$data" "$work/pass" --kdf-floor --ack-delay "$delay" \
        <"$work/$name.in" >>"$work/$name.out" 2>&1 &
    pids[$name]=$!
    exec {fd}>"$work/$name.in"
    fds[$name]=$fd
    expect "$name" '^ready$' $((ready + 1)) 60
}

# Ends node `name` with SIGKILL, as a crash would. Its traces are checked
# when it is stopped for the last time.
crash_node() {
    local name=$1 child
    child=$(pgrep -P "${pids[$name]}" | head -1)
    kill -KILL "$child"
    wait "${pids[$name]}" || true
    exec {fds[$name]}>&-
}

# The seconds since `$1`.
since() {
    echo $(($(date +%s) - $1))
}

# Asks node `name` for the sessions of identity `n` with `peer` and prints
# their number.
sessions_of() {
    local name=$1 n=$2 peer=$3 before
    before=$(count "$name" "^sessions $n $peer ")
    tell "$name" sessions "$n" "$peer"
    expect "$name" "^sessions $n $peer " $((before + 1)) 10
    last "$name" "^sessions $n $peer " | cut -d' ' -f4
}

data_ra="$work/runtime-a"
data_rb="$work/runtime-b"

echo "13. Two runtime nodes: an identity each, contacts of each other; then B goes away."
start_runtime ra "$data_ra" 0 "${tor_a[@]}"
start_runtime rb "$data_rb" 0 "${tor_b[@]}"
expect ra '^created$' 1 10
expect rb '^created$' 1 10
tell ra new-identity
tell rb new-identity
expect ra '^published 0$' 1 120
expect rb '^published 0$' 1 120
ra0=$(last ra '^identity 0 ' | cut -d' ' -f3)
rb0=$(last rb '^identity 0 ' | cut -d' ' -f3)
tell ra card 0
tell rb card 0
expect ra '^card 0 ' 1 10
expect rb '^card 0 ' 1 10
card_ra0=$(last ra '^card 0 ' | cut -d' ' -f3)
card_rb0=$(last rb '^card 0 ' | cut -d' ' -f3)
tell ra add 0 "$card_rb0"
tell rb add 0 "$card_ra0"
expect ra "^added 0 $rb0$" 1 10
expect rb "^added 0 $ra0$" 1 10
stop_node rb
# No session was needed for any of this: nothing was queued.
test "$(count ra "^session 0 $rb0$")" = 0
echo "ok   contacts of each other, no session yet"

echo "14. A queues while B is away; B comes back: delivered once, stored, acknowledged."
started=$(date +%s)
tell ra send 0 "$rb0" first
expect ra "^queued 0 $rb0 1$" 1 10
expect ra "^deferred 0 $rb0 " 1 300
echo "     deferred while B is away: $(last ra "^deferred 0 $rb0 ")"
start_runtime rb "$data_rb" 0 "${tor_b[@]}"
expect rb '^opened Clean$' 1 10
expect rb '^published 0$' 2 120
expect rb "^received 0 $ra0 1 \"first\"$" 1 900
expect ra "^delivered 0 $rb0 1$" 1 120
echo "     queued to delivered: $(since "$started")s, $(count ra "^deferred 0 $rb0 ") attempt(s) deferred"
tell rb messages 0 "$ra0"
expect rb "^messages 0 $ra0 1$" 1 10
expect rb "^message 0 $ra0 1 Received \"first\"$" 1 10
tell ra messages 0 "$rb0"
expect ra "^message 0 $rb0 1 Delivered \"first\"$" 1 10
echo "ok   B stored \"first\" once and acknowledged it; A holds it delivered"

echo "15. B crashes after storing a message and before acknowledging it: the retry is not stored again."
stop_node rb
start_runtime rb "$data_rb" 3600 "${tor_b[@]}"
expect rb '^published 0$' 3 120
started=$(date +%s)
tell ra send 0 "$rb0" second
expect ra "^queued 0 $rb0 2$" 1 10
expect rb "^received 0 $ra0 2 \"second\"$" 1 900
crash_node rb
test "$(count ra "^delivered 0 $rb0 2$")" = 0
start_runtime rb "$data_rb" 0 "${tor_b[@]}"
expect rb '^published 0$' 4 120
expect ra "^delivered 0 $rb0 2$" 1 900
echo "     queued to delivered, across the crash: $(since "$started")s"
test "$(count rb "^received 0 $ra0 2 ")" = 1
tell rb messages 0 "$ra0"
expect rb "^messages 0 $ra0 2$" 1 10
expect rb "^message 0 $ra0 1 Received \"first\"$" 2 10
expect rb "^message 0 $ra0 2 Received \"second\"$" 1 10
echo "ok   the retry after B's crash was acknowledged and not stored again"

echo "16. A crashes with a message queued: once it is back, delivery resumes."
stop_node rb
deferred=$(count ra "^deferred 0 $rb0 ")
tell ra send 0 "$rb0" third
expect ra "^queued 0 $rb0 3$" 1 10
expect ra "^deferred 0 $rb0 " $((deferred + 1)) 300
crash_node ra
published=$(count ra '^published 0$')
started=$(date +%s)
start_runtime ra "$data_ra" 0 "${tor_a[@]}"
expect ra "^identity 0 $ra0$" 2 10
start_runtime rb "$data_rb" 0 "${tor_b[@]}"
expect ra '^published 0$' $((published + 1)) 120
expect ra "^delivered 0 $rb0 3$" 1 900
echo "     restart to delivered: $(since "$started")s"
tell ra messages 0 "$rb0"
expect ra "^messages 0 $rb0 3$" 1 10
expect ra "^message 0 $rb0 1 Delivered \"first\"$" 2 10
expect ra "^message 0 $rb0 2 Delivered \"second\"$" 1 10
expect ra "^message 0 $rb0 3 Delivered \"third\"$" 1 10
test "$(count rb "^received 0 $ra0 3 ")" = 1
echo "ok   the queue survived A's crash and was delivered; earlier deliveries stayed delivered"

echo "17. Tor A restarts while a message is queued: delivery resumes through Tor."
unavailable=$(count ra '^unavailable 0$')
published=$(count ra '^published 0$')
deferred=$(count ra "^deferred 0 $rb0 Tor ")
stop_tor "$a"
expect ra '^unavailable 0$' $((unavailable + 1)) 60
started=$(date +%s)
tell ra send 0 "$rb0" fourth
expect ra "^queued 0 $rb0 4$" 1 10
expect ra "^deferred 0 $rb0 Tor " $((deferred + 1)) 300
tor -f "$a/torrc" >>"$work/tor-a-restarted.log" 2>&1 &
# Test tooling restarted it; Chutney stops it by its pid file.
echo "$!" >"$a/pid"
expect ra '^published 0$' $((published + 1)) 300
expect ra "^delivered 0 $rb0 4$" 1 900
echo "     queued to delivered, across the restart of Tor A: $(since "$started")s," \
    "$(($(count ra "^deferred 0 $rb0 Tor ") - deferred)) attempt(s) deferred for Tor"
test "$(count rb "^received 0 $ra0 4 \"fourth\"$")" = 1
echo "ok   A published its service again and delivered the queue through Tor"

echo "18. A blocks B with a message queued: nothing is delivered."
stop_node rb
deferred=$(count ra "^deferred 0 $rb0 ")
tell ra send 0 "$rb0" fifth
expect ra "^queued 0 $rb0 5$" 1 10
expect ra "^deferred 0 $rb0 " $((deferred + 1)) 300
tell ra block 0 "$rb0"
expect ra "^blocked 0 $rb0$" 1 10
# What A reported before the block is printed by now.
sleep 5
lines=$(wc -l <"$work/ra.out")
published=$(count rb '^published 0$')
start_runtime rb "$data_rb" 0 "${tor_b[@]}"
expect rb '^published 0$' $((published + 1)) 120
# Two minutes with B reachable: A dials nobody for the blocked record.
sleep 120
if tail -n +$((lines + 1)) "$work/ra.out" | grep -E "^(session|deferred|delivered) 0 $rb0"; then
    echo "FAIL A worked on the queue of a blocked contact"
    exit 1
fi
test "$(count rb "^received 0 $ra0 ")" = 4
tell ra conversations 0
expect ra "^conversation 0 $rb0 active=false messages=5 queued=1$" 1 10
echo "ok   the queued message stayed queued and was never sent"

echo "19. A second identity of A: its own queue and sessions; both sides send at once."
tell ra new-identity
expect ra '^identity 1 ' 1 120
expect ra '^published 1$' 1 120
ra1=$(last ra '^identity 1 ' | cut -d' ' -f3)
tell ra card 1
expect ra '^card 1 ' 1 10
tell rb add 0 "$(last ra '^card 1 ' | cut -d' ' -f3)"
expect rb "^added 0 $ra1$" 1 10
tell ra add 1 "$card_rb0"
expect ra "^added 1 $rb0$" 1 10
started=$(date +%s)
tell ra send 1 "$rb0" sixth
expect ra "^delivered 1 $rb0 1$" 1 900
expect rb "^received 0 $ra1 1 \"sixth\"$" 1 60
echo "     queued to delivered: $(since "$started")s"
# The session of A1 with B carries nothing of A0: A0 has none, and its
# queue is where it was.
test "$(sessions_of ra 1 "$rb0")" = 1
test "$(sessions_of ra 0 "$rb0")" = 0
tell ra conversations 0
expect ra "^conversation 0 $rb0 active=false messages=5 queued=1$" 2 10
tell rb messages 0 "$ra0"
expect rb "^messages 0 $ra0 4$" 1 10
tell rb messages 0 "$ra1"
expect rb "^messages 0 $ra1 1$" 1 10
# Both sides dial at once, once the session has lingered and ended.
expect ra "^no-session 1 $rb0$" 1 300
expect rb "^no-session 0 $ra1$" 1 300
started=$(date +%s)
tell ra send 1 "$rb0" from a at once
tell rb send 0 "$ra1" from b at once
expect rb "^received 0 $ra1 [0-9]+ \"from a at once\"$" 1 900
expect ra "^received 1 $rb0 [0-9]+ \"from b at once\"$" 1 900
expect ra "^delivered 1 $rb0 " 2 900
expect rb "^delivered 0 $ra1 " 1 900
echo "     both sending at once, to both delivered: $(since "$started")s"
test "$(count rb "\"from a at once\"$")" = 1
test "$(count ra "\"from b at once\"$")" = 1
for _ in $(seq 60); do
    if [ "$(sessions_of ra 1 "$rb0")" -le 1 ] && [ "$(sessions_of rb 0 "$ra1")" -le 1 ]; then
        break
    fi
    sleep 1
done
test "$(sessions_of ra 1 "$rb0")" -le 1
test "$(sessions_of rb 0 "$ra1")" -le 1
echo "ok   identities kept their queues and sessions apart; one session when both dialed"

echo "20. Every runtime node run contacted only loopback and Unix sockets, no DNS."
stop_node ra
stop_node rb
echo "ok   $traces node runs checked"


echo "12. Tor B goes away while B waits: the control connection is reported lost."
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
stop_tor "$b"
wait "$serve" || status=$?
cat "$work/b2.out"
test "$status" != 0
grep -q 'Tor control connection lost' "$work/b2.out"
echo "ok   B failed with the control connection lost"

echo "Private Tor network test passed."
