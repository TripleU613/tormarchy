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
IFS=$'\t' read -r cid _ exit_ip ccs <<<"$(circuit_path)"
is "the newest general circuit is the one reported" "$cid" 9
is "its path resolves hop by hop" "$ccs" "se,ch,nl"
is "the exit address is the last hop's" "$exit_ip" 10.0.0.3

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

echo
if (( failures == 0 )); then
  echo "  all helper tests passed"
else
  echo "  $failures helper test(s) failed"
  exit 1
fi
