#!/usr/bin/env bash
#
# Behaviour of the helpers in `tormarchy`, exercised rather than grepped for.
#
# The script's functions are loaded without its dispatcher, and everything that
# would touch the machine -- /proc, /run, the control port, systemctl -- is
# pointed at a scratch directory or replaced by a stand-in. What is left is the
# logic, which is where the bugs these cover actually were.

# The paths reassigned below are read by the sourced functions, which the
# linter cannot follow from here.
# shellcheck disable=SC2034

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

failures=0
ok()   { printf '  \033[32mok\033[0m    %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; failures=$(( failures + 1 )); }
skip() { printf '  \033[90m--\033[0m    %s\n' "$1"; }
is() {            # is <description> <actual> <expected>
  if [[ $2 == "$3" ]]; then ok "$1"; else bad "$1 (got '$2', wanted '$3')"; fi
}

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT

# Everything from the constants down to the dispatcher: the functions, none of
# the side effects.
# shellcheck source=/dev/null
source <(sed -n '/^# -* constants --$/,/^# -* dispatch --$/p' tormarchy)

STATE_DIR="$scratch/run"
IPV6_STATE_FILE="$STATE_DIR/ipv6-was"
PERSIST_DIR="$scratch/persist"
RECONNECT_MARKER="$PERSIST_DIR/reconnect"
BOOT_UNIT="$scratch/tormarchy-boot.service"

echo "IPv6"

if (( EUID != 0 )); then
  skip "the IPv6 helpers refuse to run unprivileged; run under sudo to cover them"
else
  IPV6_CONF="$scratch/conf"
  for iface in all default lo eth0 wlan0; do
    mkdir -p "$IPV6_CONF/$iface"
    printf '0' >"$IPV6_CONF/$iface/disable_ipv6"
  done
  # The user had switched IPv6 off on one interface themselves.
  printf '1' >"$IPV6_CONF/wlan0/disable_ipv6"
  conf() { cat "$IPV6_CONF/$1/disable_ipv6"; }

  ipv6_disable
  is "connect switches IPv6 off" "$(conf all)" 1
  is "connect keeps loopback v6, so services on ::1 keep working" "$(conf lo)" 0

  # A second disable is a mode switch while connected. It must not record the
  # state it put there itself.
  ipv6_disable
  ipv6_restore
  is "a second disable does not make restore leave IPv6 off" "$(conf all)" 0
  is "restore puts back an interface the user had switched off" "$(conf wlan0)" 1
  is "restore leaves the others on" "$(conf eth0)" 0
  [[ ! -e $IPV6_STATE_FILE ]] && ok "restore clears the saved state" || bad "restore left its state file"

  # Never connected, so nothing of ours to undo: someone who keeps IPv6 off
  # must not find it back on after a disconnect, a panic or browser-only.
  for iface in all default eth0; do printf '1' >"$IPV6_CONF/$iface/disable_ipv6"; done
  ipv6_restore
  is "restore without saved state leaves a deliberately-off IPv6 alone" "$(conf all)$(conf eth0)" 11

  # A state file written by the previous version: one bare value.
  mkdir -p "$STATE_DIR"; printf '0\n' >"$IPV6_STATE_FILE"
  printf '1' >"$IPV6_CONF/all/disable_ipv6"
  ipv6_restore
  is "restore reads the old one-value state file" "$(conf all)" 0
fi

echo
echo "Boot reconnect"

: >"$BOOT_UNIT"
remember_connected strict
is "the boot marker records the mode" "$(remembered_mode)" strict
remember_connected socks
is "browser-only is never recorded for boot" "$(remembered_mode)" strict
printf 'rm -rf /\n' >"$RECONNECT_MARKER"
is "a marker holding anything but a mode is ignored" "$(remembered_mode)" ""
forget_connected
is "forgetting clears it" "$(remembered_mode)" ""
rm -f "$BOOT_UNIT"
remember_connected lan
is "nothing is recorded while the boot receiver is off" "$(remembered_mode)" ""

echo
echo "Circuit path"

# Three general circuits out of order, plus the internal one-hop directory
# circuit that is also tagged GENERAL and must never be reported.
CIRCUITS='250+circuit-status=
7 BUILT $AAA~a,$BBB~b,$CCC~c BUILD_FLAGS=NEED_CAPACITY PURPOSE=GENERAL TIME_CREATED=2026-10-12T05:40:12.000001
9 BUILT $DDD~d,$EEE~e,$FFF~f BUILD_FLAGS=NEED_CAPACITY PURPOSE=GENERAL TIME_CREATED=2026-10-12T05:41:59.000002
8 BUILT $GGG~g,$HHH~h,$III~i BUILD_FLAGS=NEED_CAPACITY PURPOSE=GENERAL TIME_CREATED=2026-10-12T05:41:03.000003
11 BUILT $JJJ~j BUILD_FLAGS=ONEHOP_TUNNEL,IS_INTERNAL PURPOSE=GENERAL TIME_CREATED=2026-10-12T05:59:00.000000
.
250 OK'
tor_control() {
  case "$1" in
  'GETINFO circuit-status') printf '%s\n' "$CIRCUITS" ;;
  GETINFO\ ns/id/*)
    local _ i=1
    for _ in "$@"; do printf 'r relay%d x y 2026-10-12 05:00:00 10.0.0.%d 9001 0\n' "$i" "$i"; i=$(( i + 1 )); done ;;
  GETINFO\ ip-to-country/*)
    printf '250-ip-to-country/10.0.0.1=se\n250-ip-to-country/10.0.0.2=ch\n250-ip-to-country/10.0.0.3=nl\n' ;;
  esac
}
runtime_dir() { return 1; }
IFS=$'\t' read -r cid _ exit_ip ccs relays <<<"$(circuit_path)"
is "the newest general circuit is the one reported" "$cid" 9
is "its path resolves hop by hop" "$ccs" "se,ch,nl"
is "the exit address is the last hop's" "$exit_ip" 10.0.0.3
is "a reply with no s or w lines still names each relay" "$relays" "relay1//0,relay2//0,relay3//0"

# The replies as tor sends them: CRLF, and an "s" and "w" line under each "r".
tor_control() {
  case "$1" in
  'GETINFO circuit-status') printf '%s\n' "$CIRCUITS" ;;
  GETINFO\ ns/id/*)
    printf '250+ns/id/$DDD=\r\nr guardOne x y 2026-10-12 05:00:00 10.0.0.1 9001 0\r\ns Fast Guard HSDir Running Stable V2Dir Valid\r\nw Bandwidth=31200\r\n.\r\n250 OK\r\n'
    printf '250+ns/id/$EEE=\r\nr middleTwo x y 2026-10-12 05:00:00 10.0.0.2 9001 0\r\ns Fast Running Stable\r\nw Bandwidth=840 Unmeasured=1\r\n.\r\n250 OK\r\n'
    printf '250+ns/id/$FFF=\r\nr exitThree x y 2026-10-12 05:00:00 10.0.0.3 9001 0\r\ns Exit Fast Running Valid\r\nw Bandwidth=64500\r\n.\r\n250 OK\r\n' ;;
  GETINFO\ ip-to-country/*)
    printf '250-ip-to-country/10.0.0.1=se\r\n250-ip-to-country/10.0.0.2=ch\r\n250-ip-to-country/10.0.0.3=nl\r\n' ;;
  esac
}
IFS=$'\t' read -r _ _ exit_ip ccs relays <<<"$(circuit_path)"
is "each relay's nickname, flags worth showing and bandwidth come through" "$relays" \
  "guardOne/Fast Guard Stable/31200,middleTwo/Fast Stable/840,exitThree/Exit Fast/64500"
is "and the path is unchanged by them" "$ccs" "se,ch,nl"

mkdir -p "$scratch/xdg/tormarchy"; chmod 0700 "$scratch/xdg" "$scratch/xdg/tormarchy"
runtime_dir() { printf '%s' "$scratch/xdg/tormarchy"; }
printf '9\t10.0.0.3\tse,ch,nl' >"$scratch/xdg/tormarchy/path.cache"
IFS=$'\t' read -r _ _ _ _ relays <<<"$(circuit_path)"
[[ -n $relays ]] && ok "a cache line from before relay details is rebuilt, not trusted" \
  || bad "an old cache line was served without relay details"
is "the rebuilt line is marked v2" "$(cut -f1 "$scratch/xdg/tormarchy/path.cache")" v2
tor_control() { [[ $1 == 'GETINFO circuit-status' ]] && printf '%s\n' "$CIRCUITS"; }
IFS=$'\t' read -r _ _ _ ccs relays <<<"$(circuit_path)"
is "a v2 line for the same circuit is served from the cache" "$ccs|${relays%%,*}" "se,ch,nl|guardOne/Fast Guard Stable/31200"
runtime_dir() { return 1; }

echo
echo "Finding a route"

# A resolver on a free local port standing in for tor's DNSPort: it answers
# each query under the query's own id, with the rcode it is told to.
dns_port=$(python3 -c 'import socket; s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')
printf '0' >"$scratch/rcode"
python3 - "$dns_port" "$scratch/rcode" <<'PY' &
import socket, sys
s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
s.bind(("127.0.0.1", int(sys.argv[1])))
while True:
    q, addr = s.recvfrom(512)
    rcode = int(open(sys.argv[2]).read() or 0)
    if rcode < 0:
        continue                      # silence: a resolver that never answers
    s.sendto(q[:2] + bytes([0x81, 0x80 | rcode]) + q[4:], addr)
PY
resolver=$!
sleep 0.5
DNS_PORT=$dns_port
ROUTE_PROBE_TIMEOUT=2
dns_probe && ok "a lookup through tor's resolver is understood" || bad "a NOERROR answer was not accepted"
printf '2' >"$scratch/rcode"
dns_probe && bad "a SERVFAIL was taken for an answer" || ok "a resolver that fails is not a route"
printf -- '-1' >"$scratch/rcode"
dns_probe && bad "silence was taken for an answer" || ok "nor is one that never answers"
printf '0' >"$scratch/rcode"

tor_is_running() { return 0; }
sleep() { :; }
newnyms=0
tor_control() {
  case "$1" in
  'GETINFO status/circuit-established') printf '250-status/circuit-established=1\r\n250 OK\r\n' ;;
  'SIGNAL NEWNYM') newnyms=$(( newnyms + 1 )); printf '250 OK\r\n' ;;
  esac
}
# Each probe answers with the next time in the list, or fails on "x". The list
# lives in a file: find_route calls the probe in a subshell, where a shifted
# array would not stay shifted.
probe_times=()
route_probe_ms() {
  local next
  next=$(head -1 "$scratch/probes"); sed -i 1d "$scratch/probes"
  [[ -z $next || $next == x ]] && return 1
  printf '%s' "$next"
}
route() { newnyms=0; printf '%s\n' "${probe_times[@]}" >"$scratch/probes"; find_route >"$scratch/route.out" 2>&1; }

probe_times=(820)
route && [[ $newnyms == 0 ]] && ok "a fast route is taken at once, with no new circuits asked for" \
  || bad "a fast route was not taken at once ($(cat "$scratch/route.out"))"
probe_times=(6100 900)
route && [[ $newnyms == 1 ]] && ok "a slow route is swapped for fresh circuits, and the fast one taken" \
  || bad "a slow route was not retried ($newnyms new circuits: $(cat "$scratch/route.out"))"
probe_times=(x x 1200)
route && ok "routes that answer nothing are retried until one does" || bad "a dead first route gave up"
probe_times=(7000 5200 9000)
route && grep -q "5200" "$scratch/route.out" && ok "if every route is slow, the fastest is used rather than none" \
  || bad "all-slow routes did not settle on the fastest ($(cat "$scratch/route.out"))"
probe_times=(x x x)
route && bad "connect would have gone on with no route at all" || ok "no route at all is a failure, so no rules go on"
printf '2' >"$scratch/rcode"
probe_times=(500 500 500)
route && bad "a route with no working DNS was taken" || ok "a route whose DNS fails is no route"
printf '0' >"$scratch/rcode"
kill "$resolver" 2>/dev/null
unset -f sleep route_probe_ms route tor_control tor_is_running
DNS_PORT=9053

echo
echo "Exit countries"

tor_is_running() { return 0; }
tor_control() {
  case "$1" in
  'GETINFO ns/all')
    printf '250+ns/all=\r\nr a x y d t 10.0.0.1 9001 0\r\ns Exit Fast Running Valid\r\n'
    printf 'r b x y d t 10.0.0.2 9001 0\r\ns Exit Fast Running Valid\r\n'
    printf 'r c x y d t 10.0.0.3 9001 0\r\ns BadExit Exit Fast Running Valid\r\n'
    printf 'r d x y d t 10.0.0.4 9001 0\r\ns Exit Fast Running Valid\r\n.\r\n250 OK\r\n' ;;
  GETINFO\ ip-to-country/*)
    printf '250-ip-to-country/10.0.0.1=de\r\n250-ip-to-country/10.0.0.2=DE\r\n250-ip-to-country/10.0.0.4=??\r\n250 OK\r\n250 closing connection\r\n' ;;
  esac
}
is "exit --list prints bare country codes, each once" "$(cmd_exit --list | tr '\n' ' ')" "de "

TORRC_DIR="$scratch/torrc.d"; mkdir -p "$TORRC_DIR"
TORRC_EXIT="$TORRC_DIR/10-tormarchy-exit.conf"
TORRC_EXCLUDE="$TORRC_DIR/11-tormarchy-exclude.conf"
require_root() { :; }
tor_is_running() { return 1; }
( cmd_exit --exclude "us, RU,us" >/dev/null )
is "--exclude writes each country once, lowercased, in tor's syntax" \
  "$(grep -v '^#' "$TORRC_EXCLUDE")" "ExcludeExitNodes {us},{ru}"
is "and reads back as the list" "$(excluded_exits | tr '\n' ' ')" "us ru "
( cmd_exit --exclude "u5" >/dev/null 2>&1 ) && bad "a malformed code was accepted" \
  || ok "a malformed code is refused"
is "and the refusal changed nothing" "$(grep -v '^#' "$TORRC_EXCLUDE")" "ExcludeExitNodes {us},{ru}"
( cmd_exit us >/dev/null 2>&1 ) && bad "exits were pinned to an excluded country" \
  || ok "pinning exits to an excluded country is refused"
( cmd_exit --exclude none >/dev/null )
[[ ! -e $TORRC_EXCLUDE ]] && ok "--exclude none removes the file" || bad "--exclude none left $TORRC_EXCLUDE"
( cmd_exit de >/dev/null )
( cmd_exit --exclude de,us >/dev/null 2>&1 ) && bad "the pinned country was excluded" \
  || ok "excluding the pinned country is refused, since it would leave no exit"
[[ ! -e $TORRC_EXCLUDE ]] && ok "and nothing was written" || bad "a conflicting exclusion was written"
rm -f "$TORRC_EXIT"

echo
echo "Bridges"

check=$(printf 'obfs4 192.0.2.1:443 AAAA cert=x iat-mode=0\r\nBridge obfs4 192.0.2.2:443 BBBB cert=y iat-mode=0\n  webtunnel [2001:db8::1]:443 CCCC url=https://example.com/x\n198.51.100.7:443 DDDD\n# a comment\n\nthis is not a bridge\nobfs4 not-an-address\n' | cmd_bridge check)
python3 -c 'import json,sys; json.loads(sys.argv[1])' "$check" 2>/dev/null \
  && ok "bridge check prints valid JSON" || bad "bridge check printed: $check"
is "it counts every valid line, CRLF, a Bridge prefix and indentation included" \
  "$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["count"])' "$check")" 4
is "it counts the lines that are not bridge lines" \
  "$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["invalidLines"])' "$check")" 2
is "it names each transport, plain bridges included" \
  "$(python3 -c 'import json,sys; print(",".join(t["name"]+":"+str(t["count"]) for t in json.loads(sys.argv[1])["transports"]))' "$check")" \
  "obfs4:2,plain:1,webtunnel:1"
[[ $check != *192.0.2* && $check != *AAAA* && $check != *example.com* ]] \
  && ok "nothing of the lines themselves comes back" || bad "bridge check echoed a line: $check"
is "empty input is no bridges, not an error" "$(cmd_bridge check </dev/null)" '{"invalidLines":0,"count":0,"transports":[]}'

if (( EUID == 0 )); then
  BRIDGE_STASH="$scratch/etc-tormarchy/bridges.stash"
  BRIDGE_SUMMARY="$scratch/etc-tormarchy/bridges.summary"
  TORRC_BRIDGES="$scratch/20-tormarchy-bridges.conf"
  mkdir -p "$scratch/etc-tormarchy"
  printf 'obfs4 192.0.2.1:443 AAAA cert=x iat-mode=0\nobfs4 192.0.2.2:443 BBBB cert=y iat-mode=0\n' >"$BRIDGE_STASH"
  write_bridge_summary
  is "the summary the panel reads is transport counts" "$(cat "$BRIDGE_SUMMARY")" "obfs4 2"
  is "and is readable by the panel" "$(stat -c %a "$BRIDGE_SUMMARY")" 644
  is "bridge status --json reads it" "$(cmd_bridge status --json)" \
    "{\"state\":\"stashed\",\"count\":2,\"transports\":[{\"name\":\"obfs4\",\"count\":2,\"available\":$(transport_binary obfs4 >/dev/null 2>&1 && echo true || echo false),\"installHint\":\"$(install_hint obfs4)\"}]}"
  rm -f "$BRIDGE_STASH"; write_bridge_summary
  [[ ! -e $BRIDGE_SUMMARY ]] && ok "with no saved bridges the summary goes too" || bad "a summary outlived its bridges"
else
  skip "the bridge summary (needs root)"
fi

echo
echo "Doctor"

command() { [[ $1 == -v && ${2:-} == tor ]] && return 0; builtin command "$@"; }
MODE_FILE="$STATE_DIR/mode"; mkdir -p "$STATE_DIR"; printf 'lan\n' >"$MODE_FILE"
rules_present() { return 0; }
tor_is_running() { return 0; }
bootstrap_progress() { printf '100'; }
cookie_path() { printf '/run/tor/control.authcookie'; }
timedatectl() { printf 'yes\n'; }
getent() { return 0; }
curl() {
  case "$*" in
  *-6*) return 7 ;;
  *check.torproject.org*) printf '{"IsTor":true,"IP":"185.220.101.4"}' ;;
  *) return 7 ;;
  esac
}
json=$(cmd_doctor --json); rc=$?
python3 -c 'import json,sys; json.loads(sys.argv[1])' "$json" 2>/dev/null \
  && ok "doctor --json is valid JSON" || bad "doctor --json printed: $json"
[[ $json != *$'\033'* ]] && ok "and carries no terminal colour codes" || bad "doctor --json has escape codes in it"
is "every check carries the id the panel names it by" \
  "$(python3 -c 'import json,sys; print(",".join(c["id"] for c in json.loads(sys.argv[1])["checks"]))' "$json" 2>/dev/null)" \
  "tor,bootstrap,control,exit,ipv6,ipv6-stack,dns,forwarding,clock"
is "a clean run reports ok" "$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); print(d["ok"], d["failures"])' "$json" 2>/dev/null)" "True 0"
is "and exits 0" "$rc" 0
is "the exit address is a check's detail" \
  "$(python3 -c 'import json,sys; print([c["detail"] for c in json.loads(sys.argv[1])["checks"] if "confirms" in c["text"]][0])' "$json" 2>/dev/null)" \
  "exit as seen from outside: 185.220.101.4"
curl() {
  case "$*" in
  *-6*) return 0 ;;
  *check.torproject.org*) printf '{"IsTor":false,"IP":"203.0.113.9"}' ;;
  esac
}
json=$(cmd_doctor --json); rc=$?
is "a leak is a failure, counted" "$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); print(d["ok"], d["failures"])' "$json" 2>/dev/null)" "False 2"
[[ $rc != 0 ]] && ok "and the exit status says so" || bad "doctor --json exited 0 with failures"
printf 'socks\n' >"$MODE_FILE"
rules_present() { return 1; }
curl() { [[ $* == *--socks5-hostname* ]] && printf '{"IsTor":true,"IP":"185.220.101.4"}'; }
json=$(cmd_doctor --json)
is "browser-only checks the proxy itself rather than skipping routing" \
  "$(python3 -c 'import json,sys; print([c["status"] for c in json.loads(sys.argv[1])["checks"] if "SOCKS proxy" in c["text"]][0])' "$json" 2>/dev/null)" pass
unset -f curl timedatectl getent

echo
echo "The privilege boundary"

# The real script, as pkexec would run it for the passwordless grant. Every
# subcommand not on PKEXEC_COMMANDS must be refused before it does anything;
# browser-exec once became whatever it was handed, as root.
mapfile -t subcommands < <(sed -n '/^case "$command" in$/,/^esac$/p' tormarchy \
  | sed -n 's/^\([a-z][a-z-]*\)\(  *| [a-z|-]*\)\{0,1\})[[:space:]].*/\1/p')
(( ${#subcommands[@]} > 15 )) && ok "found the dispatcher's ${#subcommands[@]} subcommands" \
  || bad "could not read the dispatcher's subcommands (${subcommands[*]})"
refused_all=1
for sub in "${subcommands[@]}"; do
  [[ " ${PKEXEC_COMMANDS[*]} " == *" $sub "* ]] && continue
  out=$(PKEXEC_UID=$(id -u) ./tormarchy "$sub" -- touch "$scratch/pwned-$sub" 2>&1)
  if [[ $? == 0 || -e $scratch/pwned-$sub || $out != *"does not run as root through polkit"* ]]; then
    bad "under pkexec, '$sub' was not refused: $out"; refused_all=0
  fi
done
(( refused_all )) && ok "under pkexec, every subcommand that is not on the list is refused"
out=$(PKEXEC_UID=$(id -u) ./tormarchy connect --dry-run lan 2>/dev/null)
[[ $out == *"table inet tormarchy"* ]] && ok "one that is on it still runs" || bad "connect --dry-run was refused under pkexec"
if (( EUID == 0 )); then
  ./tormarchy browser-exec -- touch "$scratch/pwned-root" >/dev/null 2>&1
  [[ ! -e $scratch/pwned-root ]] && ok "browser-exec run as root starts nothing" \
    || bad "browser-exec ran its command as root"
  out=$(./tormarchy browser firefox 2>&1)
  [[ $out == *"not as root"* ]] && ok "a browser is never launched as root" || bad "a browser launch as root was not refused: $out"
else
  skip "browser-exec as root (needs root)"
fi
check=$(printf 'obfs4 192.0.2.1:443 AAAA cert=x \\\nobfs4 192.0.2.2:443 BBBB cert=y\001\n' | cmd_bridge check)
is "a bridge line with a backslash or a control character is refused" \
  "$(python3 -c 'import json,sys; d=json.loads(sys.argv[1]); print(d["invalidLines"], d["count"])' "$check")" "2 0"

echo
echo "Status"

TORRC_BASE="$scratch/torrc-base"; : >"$TORRC_BASE"
TORRC_EXIT="$scratch/torrc-exit"
MODE_FILE="$STATE_DIR/mode"; ACTIVE_FILE="$STATE_DIR/active"
mkdir -p "$STATE_DIR"
command() { [[ $1 == -v && ${2:-} == tor ]] && return 0; builtin command "$@"; }

# Rules on, tor gone: the kill switch holding the network shut.
tor_is_running() { return 1; }
rules_present() { return 0; }
printf 'strict\n' >"$MODE_FILE"
json=$(cmd_status --json)
python3 -c 'import json,sys; json.loads(sys.argv[1])' "$json" 2>/dev/null \
  && ok "status --json is valid JSON" || bad "status --json is not valid JSON: $json"
text=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["statusText"])' "$json" 2>/dev/null)
[[ $text == *"not running"* ]] && ok "a held kill switch is not reported as traffic through Tor" \
  || bad "a held kill switch read as: $text"

# tor up, control port unreachable: no bootstrap figure exists to report.
tor_is_running() { return 0; }
rules_present() { return 1; }
tor_vitals() { printf '0\t0\t0'; return 1; }
printf 'socks\n' >"$MODE_FILE"
json=$(cmd_status --json)
cp=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["controlPort"])' "$json" 2>/dev/null)
is "an unreachable control port is reported as such" "$cp" False
text=$(python3 -c 'import json,sys; print(json.loads(sys.argv[1])["statusText"])' "$json" 2>/dev/null)
[[ $text != *"0%"* ]] && ok "and not as bootstrapping at 0%" || bad "status claimed: $text"

# Connected, with relay details and an exclusion: what the panel's hop
# tooltips and exit picker read.
tor_vitals() { printf '100\t10\t20'; }
rules_present() { return 0; }
printf 'lan\n' >"$MODE_FILE"
circuit_path() { printf '9\t42\t10.0.0.3\tse,ch,nl\tguardOne/Fast Guard Stable/31200,middleTwo//0,exitThree/Exit Fast/64500'; }
printf 'ExcludeExitNodes {us},{ru}\n' >"$TORRC_EXCLUDE"
json=$(cmd_status --json)
python3 -c 'import json,sys; json.loads(sys.argv[1])' "$json" 2>/dev/null \
  && ok "status --json with hops is valid JSON" || bad "status --json with hops is not valid JSON: $json"
is "each hop carries its relay" \
  "$(python3 -c 'import json,sys; h=json.loads(sys.argv[1])["hops"]; print(h[0]["nickname"], "+".join(h[0]["flags"]), h[0]["bandwidthKBps"], h[1]["flags"], h[2]["cc"])' "$json" 2>/dev/null)" \
  "guardOne Fast+Guard+Stable 31200 [] nl"
is "the path is still there for older panels" "$(python3 -c 'import json,sys; print(",".join(json.loads(sys.argv[1])["path"]))' "$json" 2>/dev/null)" "se,ch,nl"
is "excluded exits are listed" "$(python3 -c 'import json,sys; print(",".join(json.loads(sys.argv[1])["excludedExits"]))' "$json" 2>/dev/null)" "us,ru"
rm -f "$TORRC_EXCLUDE"

echo
if (( failures == 0 )); then
  echo "  all helper tests passed"
else
  echo "  $failures helper test(s) failed"
  exit 1
fi
