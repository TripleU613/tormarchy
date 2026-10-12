#!/usr/bin/env bash
#
# The kill switch in a real kernel, not just a ruleset nft will parse.
#
#   sudo tests/kernel/killswitch.sh <ruleset.nft>
#
# Two network namespaces joined by a veth: a client, where the ruleset goes on,
# and a remote host on an address outside every LAN range, so it is "the
# internet" in lan mode too. A connection is opened before the rules, as a
# stream would be, and then the rules go on. What it proves:
#
#   - not one packet from the client reaches the wire afterwards: counted by
#     nftables on the remote side, so neighbour discovery cannot blur it;
#   - the stream opened before the switch fails at once, rather than hanging
#     on its own timeout, so the app reconnects -- through Tor -- right away;
#   - UDP (QUIC, say) fails at once too, and a new connection is redirected
#     to tor's port instead of going out.
#
# Exit 0 when all of that holds, 1 when any of it does not, 2 when the machine
# cannot make namespaces (no root, a restricted container) -- a skip, not a
# failure of the ruleset.

set -uo pipefail
ruleset=${1:?usage: killswitch.sh <ruleset.nft>}
(( EUID == 0 )) || exit 2
command -v ip >/dev/null 2>&1 && command -v nft >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1 || exit 2

client=tmks-client-$$
remote=tmks-remote-$$
ip netns add "$client" 2>/dev/null || exit 2
ip netns add "$remote" 2>/dev/null || { ip netns del "$client"; exit 2; }
cleanup() {
  ip netns pids "$remote" 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns pids "$client" 2>/dev/null | xargs -r kill 2>/dev/null
  ip netns del "$client" 2>/dev/null
  ip netns del "$remote" 2>/dev/null
}
trap cleanup EXIT

C=(ip netns exec "$client"); R=(ip netns exec "$remote")
ip link add "tmc$$" type veth peer name "tmr$$" || exit 2
ip link set "tmc$$" netns "$client"; ip link set "tmr$$" netns "$remote"
"${C[@]}" ip addr add 198.51.100.1/24 dev "tmc$$"; "${C[@]}" ip link set "tmc$$" up; "${C[@]}" ip link set lo up
"${R[@]}" ip addr add 198.51.100.2/24 dev "tmr$$"; "${R[@]}" ip link set "tmr$$" up; "${R[@]}" ip link set lo up

# The remote's view: a counter on every IP packet the client sends it.
"${R[@]}" nft -f - <<'NFT' || exit 2
table inet wire {
  counter from_client { }
  chain in { type filter hook input priority 0; policy accept; ip saddr 198.51.100.1 counter name "from_client"; }
}
NFT
"${R[@]}" python3 -c '
import socket, threading
srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
srv.bind(("0.0.0.0", 8080)); srv.listen()
def echo(c):
    while (d := c.recv(64)):
        c.sendall(d)
while True:
    threading.Thread(target=echo, args=(srv.accept()[0],), daemon=True).start()
' &
sleep 0.5

result=$("${C[@]}" python3 - "$ruleset" "$remote" <<'PY'
import socket, subprocess, sys, time

def on_the_wire():
    out = subprocess.run(["ip", "netns", "exec", sys.argv[2], "nft", "list", "counter", "inet", "wire", "from_client"],
                         capture_output=True, text=True, check=True).stdout
    return int(out.split("packets ")[1].split()[0])

stream = socket.create_connection(("198.51.100.2", 8080), timeout=3)
stream.sendall(b"before"); assert stream.recv(64) == b"before"
quic = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); quic.connect(("198.51.100.2", 443))
time.sleep(0.2)
before = on_the_wire()
subprocess.run(["nft", "-f", sys.argv[1]], check=True)

def timed(fn):
    t = time.monotonic()
    try:
        fn()
        return "answered", time.monotonic() - t
    except OSError as e:
        return type(e).__name__, time.monotonic() - t

stream.settimeout(20); quic.settimeout(20)
def old_stream():
    stream.sendall(b"after")
    if not stream.recv(64): raise ConnectionResetError("closed")
print("stream %s %.2f" % timed(old_stream))
print("udp %s %.2f" % timed(lambda: (quic.send(b"x"), quic.recv(64))))
print("new %s %.2f" % timed(lambda: socket.create_connection(("198.51.100.2", 8080), timeout=10)))
time.sleep(1)
print("wire %d" % (on_the_wire() - before))
PY
)
echo "$result" | sed 's/^/    /'

ok=1
grep -q '^wire 0$' <<<"$result" || ok=0
# Failed, and fast: under a second, where an app left hanging waits tens.
awk '$1 == "stream" || $1 == "udp" || $1 == "new" { if ($2 == "answered" || $3 + 0 >= 1) bad = 1 } END { exit bad }' <<<"$result" || ok=0
(( ok ))
