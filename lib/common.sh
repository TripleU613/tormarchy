#!/bin/bash
# Shared constants and helpers for the tormarchy-* commands.
# Installed to /usr/local/lib/tormarchy/common.sh and sourced by absolute path,
# because pkexec scrubs the environment and cannot be trusted to find a sibling.

TOR_USER="tor"
TRANS_PORT=9040
DNS_PORT=9053
SOCKS_PORT=9050
CONTROL_PORT=9051

# Tor's virtual address range for .onion names resolved via AutomapHostsOnResolve.
VIRT_NET="10.192.0.0/10"

NFT_TABLE="tormarchy"
STATE_DIR="/run/tormarchy"
MODE_FILE="$STATE_DIR/mode"
ACTIVE_FILE="$STATE_DIR/active"

TORRC="/etc/tor/torrc"
TORRC_DIR="/etc/tor/torrc.d"
TORRC_BASE="$TORRC_DIR/00-tormarchy.conf"
TORRC_EXIT="$TORRC_DIR/10-tormarchy-exit.conf"
TORRC_BRIDGES="$TORRC_DIR/20-tormarchy-bridges.conf"

# Bridge lines are kept even while bridges are off, so a stalled direct
# bootstrap can fall back to them without asking for them again.
BRIDGE_STASH="/etc/tormarchy/bridges.stash"

# Where to look for a control cookie, best first.
#
# The published copy comes first because it is the only one that works without
# a fresh login. Group membership reaches a process at login and never after,
# so on the session where tormarchy was installed -- the session that is going
# to use it -- the group-readable original at /var/lib/tor is unreadable no
# matter how correct the permissions are. Every privileged command that starts
# or restarts tor publishes a copy owned by the invoking user instead, which
# grants exactly what joining the group would have granted, immediately.
USER_COOKIE="/run/tormarchy/control.cookie"
COOKIE_CANDIDATES=("$USER_COOKIE" /var/lib/tor/control_auth_cookie /run/tor/control_auth_cookie)

cookie_path() {
  local candidate
  for candidate in "${COOKIE_CANDIDATES[@]}"; do
    [[ -r $candidate ]] && { printf '%s' "$candidate"; return 0; }
  done
  # Report the expected location so an error message can name a real path.
  printf '%s' "${COOKIE_CANDIDATES[0]}"
  return 1
}

# Why the cookie is unreadable, in the user's terms. There are two very
# different causes and telling them apart matters: "join the tor group" is
# useless advice to someone already in it, which is the usual case -- group
# membership only reaches a process at login, so every session started before
# setup ran is locked out while `getent group tor` happily lists the user.
cookie_hint() {
  local user="${SUDO_USER:-${USER:-$(id -un)}}"

  if ! getent group tor >/dev/null 2>&1; then
    printf 'the tor group does not exist -- is tor installed?'
    return
  fi

  if ! id -nG "$user" 2>/dev/null | tr ' ' '\n' | grep -qx tor; then
    printf "%s is not in the 'tor' group. Run setup, then log out and back in." "$user"
    return
  fi

  # In the group on paper but not in this process, which is the normal case and
  # no longer needs fixing by hand: connecting publishes a copy this user owns.
  if ! id -G 2>/dev/null | tr ' ' '\n' | grep -qx "$(getent group tor | cut -d: -f3)"; then
    if tor_is_running; then
      printf 'no readable cookie has been published yet. Reconnect (or run tormarchy-connect) and it will appear.'
    else
      printf 'tor is not running, so there is no control cookie to read yet.'
    fi
    return
  fi

  printf 'the cookie exists but is not readable; check the mode of %s and /var/lib/tor.' "${COOKIE_CANDIDATES[0]}"
}

VALID_MODES=(lan strict socks)

# Networks that stay reachable in lan mode. Link-local and CGNAT are included
# because captive portals and tethered connections live there.
LAN_NETS_V4="10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 100.64.0.0/10"

die() {
  echo "${0##*/}: $*" >&2
  exit 1
}

is_valid_mode() {
  local candidate="${1:-}" mode
  for mode in "${VALID_MODES[@]}"; do
    [[ $candidate == "$mode" ]] && return 0
  done
  return 1
}

# sudo when there's a terminal to type into, pkexec otherwise. Called from the
# shell there is no tty, so this surfaces Omarchy's own polkit dialog instead
# of silently failing or spawning a terminal. Same shape as omarchy-dns.
require_root() {
  (( EUID == 0 )) && return 0
  local self
  self=$(readlink -f "$0")
  if [[ -t 0 ]]; then
    exec sudo "$self" "$@"
  else
    exec pkexec "$self" "$@"
  fi
}

# The human behind this invocation, whether we got here through sudo or pkexec.
invoking_user() {
  if [[ -n ${PKEXEC_UID:-} ]]; then
    id -un "$PKEXEC_UID" 2>/dev/null && return 0
  fi
  [[ -n ${SUDO_USER:-} ]] && { printf '%s' "$SUDO_USER"; return 0; }
  return 1
}

# Root-only. Copy tor's freshly written control cookie somewhere the invoking
# user can read it. Call this after any start or restart of tor: the cookie is
# regenerated each time, and a stale copy authenticates against nothing.
#
# This hands one user the ability to drive tor's control port, which is the same
# authority that being in the 'tor' group confers. The difference is that it
# takes effect now rather than after a logout.
publish_cookie() {
  (( EUID == 0 )) || return 1

  local user source attempt
  user=$(invoking_user) || return 1

  # tor writes the cookie while processing its config, which can land after
  # "systemctl start" has already returned. Give it a moment rather than
  # publishing nothing and leaving the panel's stats permanently empty.
  for (( attempt = 0; attempt < 50; attempt++ )); do
    source=$(cookie_path) && break
    sleep 0.1
  done
  [[ -n ${source:-} && -r $source ]] || return 1
  [[ $source != "$USER_COOKIE" ]] || return 0

  install -d -m 0755 "$STATE_DIR" || return 1
  install -o "$user" -g "$user" -m 0600 "$source" "$USER_COOKIE" 2>/dev/null || return 1
}

current_mode() {
  if [[ -r $MODE_FILE ]]; then
    local mode
    mode=$(<"$MODE_FILE")
    is_valid_mode "$mode" && { printf '%s' "$mode"; return 0; }
  fi
  printf ''
}

# "Connected" means a ruleset is actually installed, not merely that tor is up.
# The state file is the unprivileged view; when we are root we trust nft and
# repair the file, so an externally flushed table cannot leave us claiming
# protection we no longer have.
rules_present() {
  if (( EUID == 0 )); then
    nft list table inet "$NFT_TABLE" >/dev/null 2>&1
    return $?
  fi
  [[ -e $ACTIVE_FILE ]]
}

tor_is_running() {
  systemctl is-active --quiet tor.service 2>/dev/null
}

# One control-port session, many GETINFOs. Opening a socket per question would
# triple the cost of a status poll that runs every few seconds.
#
# Uses bash's /dev/tcp rather than nc: netcat is not guaranteed to be installed
# on Arch, and bash is.
tor_control() {
  local cookie_file
  cookie_file=$(cookie_path) || return 1

  local cookie
  cookie=$(od -An -tx1 -v "$cookie_file" 2>/dev/null | tr -d ' \n') || return 1
  [[ -n $cookie ]] || return 1

  exec 3<>"/dev/tcp/127.0.0.1/$CONTROL_PORT" 2>/dev/null || return 1

  local reply cmd
  printf 'AUTHENTICATE %s\r\n' "$cookie" >&3
  if ! IFS= read -r -t 3 reply <&3 || [[ $reply != 250* ]]; then
    exec 3<&- 2>/dev/null
    exec 3>&- 2>/dev/null
    return 1
  fi

  for cmd in "$@"; do
    printf '%s\r\n' "$cmd" >&3
  done
  printf 'QUIT\r\n' >&3

  # Tor closes the connection after QUIT, so this terminates on its own. The
  # timeout is a guard against a wedged daemon, not a normal code path; callers
  # that pipeline hundreds of commands raise it.
  timeout "${TOR_CONTROL_TIMEOUT:-5}" cat <&3
  local status=$?

  exec 3<&- 2>/dev/null
  exec 3>&- 2>/dev/null
  return $status
}

json_escape() {
  local text="${1:-}"
  text=${text//\\/\\\\}
  text=${text//\"/\\\"}
  text=${text//$'\t'/ }
  text=${text//$'\r'/}
  text=${text//$'\n'/ }
  printf '%s' "$text"
}

# Emit the nftables ruleset for a mode. Everything lives in our own table so
# the user's ufw rules are never read, rewritten, or flushed: a DROP in any
# table wins, so the kill switch holds without us touching theirs.
build_ruleset() {
  local mode="$1"
  local lan_rules_nat="" lan_rules_filter=""

  if [[ $mode == lan ]]; then
    lan_rules_nat="    ip daddr { $LAN_NETS_V4 } return"
    lan_rules_filter="    ip daddr { $LAN_NETS_V4 } accept"
  fi

  cat <<RULES
table inet $NFT_TABLE {
  chain nat_output {
    type nat hook output priority dstnat; policy accept;

    # Tor's own traffic must never be redirected back into Tor.
    meta skuid "$TOR_USER" return
    oifname "lo" return
    ip daddr 127.0.0.0/8 return

    # Onion addresses land in Tor's virtual range. This MUST come before the
    # LAN exemption below: 10.192.0.0/10 sits inside 10.0.0.0/8, so a LAN
    # return placed first would send every .onion straight out of the NIC.
    ip daddr $VIRT_NET meta l4proto tcp redirect to :$TRANS_PORT

    # DHCP has to survive or the lease expires and the link dies with it.
    # A machine with no address is not privacy, just breakage.
    udp sport 68 udp dport 67 return

    # DNS goes to Tor's resolver BEFORE the LAN exemption below. Ordering these
    # the other way round is a live DNS leak: the usual resolver on a home
    # network is the router itself, which the LAN rule would return, sending
    # every lookup out in cleartext while the bar still showed a lit onion.
    # The cost is that LAN hostname resolution stops working in lan mode --
    # the right trade, and the reason mDNS is dropped further down too.
    meta l4proto { tcp, udp } th dport 53 redirect to :$DNS_PORT

$lan_rules_nat

    # Everything else over TCP goes to the transparent port.
    meta l4proto tcp redirect to :$TRANS_PORT
  }

  chain filter_output {
    type filter hook output priority filter; policy drop;

    meta skuid "$TOR_USER" accept

    # Redirected packets arrive here with a loopback destination, which is
    # what makes the default-drop below safe for real traffic.
    oifname "lo" accept
    ip daddr 127.0.0.0/8 accept

    udp sport 68 udp dport 67 accept

$lan_rules_filter

    # Everything else: UDP, QUIC, ICMP, and all IPv6. This is the kill switch.
    counter drop
  }
}
RULES
}

# Root-only. Replaces the table atomically: the delete and the new ruleset go
# in as one nft transaction, so there is no instant where traffic is unfiltered.
apply_rules() {
  local mode="$1" ruleset
  ruleset=$(printf 'table inet %s\ndelete table inet %s\n%s\n' \
    "$NFT_TABLE" "$NFT_TABLE" "$(build_ruleset "$mode")")

  # Check before commit so a malformed ruleset can never leave a half state.
  printf '%s\n' "$ruleset" | nft -c -f - || return 1
  printf '%s\n' "$ruleset" | nft -f - || return 1
}

flush_rules() {
  # The add-then-delete dance makes this idempotent: deleting a table that
  # does not exist is an error, so create an empty one first.
  nft "add table inet $NFT_TABLE" 2>/dev/null || true
  nft "delete table inet $NFT_TABLE" 2>/dev/null || true
}

write_state() {
  local mode="$1" active="$2"
  install -d -m 0755 "$STATE_DIR"
  printf '%s\n' "$mode" >"$MODE_FILE"
  chmod 0644 "$MODE_FILE"
  if [[ $active == yes ]]; then
    : >"$ACTIVE_FILE"
    chmod 0644 "$ACTIVE_FILE"
  else
    rm -f "$ACTIVE_FILE"
  fi
}

# Bootstrap, plus the session traffic counters, in one control session. These
# three get asked for together on every poll, so they travel together.
# Prints "<pct>\t<bytes-read>\t<bytes-written>".
tor_vitals() {
  local out pct read written
  if ! out=$(tor_control 'GETINFO status/bootstrap-phase' 'GETINFO traffic/read' 'GETINFO traffic/written' 2>/dev/null); then
    printf '0\t0\t0'
    return 1
  fi
  pct=$(grep -o 'PROGRESS=[0-9]\{1,3\}' <<<"$out" | head -1 | cut -d= -f2)
  read=$(sed -n 's/^250[-+]traffic\/read=\([0-9]*\).*/\1/p' <<<"$out" | head -1)
  written=$(sed -n 's/^250[-+]traffic\/written=\([0-9]*\).*/\1/p' <<<"$out" | head -1)
  printf '%s\t%s\t%s' "${pct:-0}" "${read:-0}" "${written:-0}"
}

# The full three-hop path of the newest general circuit, as country codes:
# "<circuit-id>\t<age-seconds>\t<exit-ip>\t<cc>,<cc>,<cc>".
#
# Resolving a path costs a fingerprint lookup and a GeoIP lookup per hop, so the
# result is cached against the circuit id in the user's runtime dir. A path only
# changes when the circuit does, and re-deriving it every few seconds would put
# pointless load on the control port for an answer that has not moved.
circuit_path() {
  local cache="${XDG_RUNTIME_DIR:-/tmp}/tormarchy-path.cache"
  local circuits line id created path

  circuits=$(tor_control 'GETINFO circuit-status' 2>/dev/null) || return 1

  line=""
  while IFS= read -r candidate; do
    [[ $candidate == *" BUILT "* ]] || continue
    [[ $candidate == *"PURPOSE=GENERAL"* ]] || continue
    line=$candidate
  done <<<"$circuits"
  [[ -n $line ]] || return 1

  id=$(awk '{print $1}' <<<"$line")
  created=$(grep -o 'TIME_CREATED=[^ ]*' <<<"$line" | head -1 | cut -d= -f2)
  path=$(awk '{print $3}' <<<"$line")

  local age=0 now created_ts
  if [[ -n $created ]]; then
    now=$(date -u +%s)
    created_ts=$(date -u -d "${created%%.*}Z" +%s 2>/dev/null) || created_ts=""
    [[ -n $created_ts ]] && age=$(( now - created_ts ))
    (( age < 0 )) && age=0
  fi

  # Cache hit: same circuit id, so the hops and the exit address are unchanged.
  if [[ -r $cache ]]; then
    local cached_id cached_ip cached_ccs
    IFS=$'\t' read -r cached_id cached_ip cached_ccs <"$cache"
    if [[ $cached_id == "$id" && -n $cached_ccs ]]; then
      printf '%s\t%s\t%s\t%s' "$id" "$age" "$cached_ip" "$cached_ccs"
      return 0
    fi
  fi

  # Fingerprints in path order.
  local -a fps=() hop
  local IFS_SAVE=$IFS
  IFS=','
  for hop in $path; do
    hop=${hop%%~*}
    hop=${hop#\$}
    [[ -n $hop ]] && fps+=("$hop")
  done
  IFS=$IFS_SAVE
  (( ${#fps[@]} > 0 )) || return 1

  # One session for every fingerprint. Each ns/id reply carries a single "r "
  # line, and replies come back in request order, so the addresses line up with
  # the hops without needing to be matched back up.
  local -a queries=() ips=()
  local fp
  for fp in "${fps[@]}"; do queries+=("GETINFO ns/id/\$$fp"); done
  local ns
  ns=$(tor_control "${queries[@]}" 2>/dev/null) || return 1
  mapfile -t ips < <(awk '/^r /{print $7}' <<<"$ns")
  (( ${#ips[@]} > 0 )) || return 1

  # And one session for the countries. These replies name the address in the
  # key, so they are matched rather than trusted to arrive in order.
  queries=()
  local ip
  for ip in "${ips[@]}"; do queries+=("GETINFO ip-to-country/$ip"); done
  local geo
  geo=$(tor_control "${queries[@]}" 2>/dev/null) || return 1

  local ccs="" cc
  for ip in "${ips[@]}"; do
    cc=$(sed -n "s|^250[-+]ip-to-country/${ip}=||p" <<<"$geo" | head -1 | tr -d '\r')
    [[ -z $cc || $cc == "??" ]] && cc="xx"
    ccs+="${ccs:+,}$cc"
  done

  local exit_ip="${ips[-1]}"
  printf '%s\t%s\t%s' "$id" "$exit_ip" "$ccs" >"$cache" 2>/dev/null || true
  printf '%s\t%s\t%s\t%s' "$id" "$age" "$exit_ip" "$ccs"
}

# Bootstrap percentage, 0-100. Prints 0 and fails when the control port is
# unreachable, so callers can distinguish "not up" from "up at 0%".
bootstrap_progress() {
  local out pct
  if ! out=$(tor_control 'GETINFO status/bootstrap-phase' 2>/dev/null); then
    printf '0'
    return 1
  fi
  pct=$(printf '%s' "$out" | grep -o 'PROGRESS=[0-9]\{1,3\}' | head -1 | cut -d= -f2)
  printf '%s' "${pct:-0}"
}

# Exit relay details for the newest general-purpose circuit, as
# "<country-code>\t<ip>\t<age-seconds>". All of it comes from Tor's own
# control port: no request to check.torproject.org, so a status poll costs
# nothing on the wire and leaks no polling pattern to a third party.
exit_info() {
  local circuits fp created ns ip cc age now

  circuits=$(tor_control 'GETINFO circuit-status' 2>/dev/null) || return 1

  # Last BUILT general circuit wins -- that is the one carrying new streams.
  local line=""
  while IFS= read -r candidate; do
    [[ $candidate == *" BUILT "* ]] || continue
    [[ $candidate == *"PURPOSE=GENERAL"* ]] || continue
    line=$candidate
  done <<<"$circuits"
  [[ -n $line ]] || return 1

  # Path is field 3: "$FP~nick,$FP~nick,$FP~nick". The exit is the last hop.
  local path last
  path=$(awk '{print $3}' <<<"$line")
  last=${path##*,}
  fp=${last%%~*}
  fp=${fp#\$}
  [[ -n $fp ]] || return 1

  created=$(grep -o 'TIME_CREATED=[^ ]*' <<<"$line" | head -1 | cut -d= -f2)

  ns=$(tor_control "GETINFO ns/id/\$$fp" 2>/dev/null) || return 1
  # Router status line: r nickname identity digest date time IP ORPort DirPort
  ip=$(awk '/^r /{print $7; exit}' <<<"$ns")
  [[ -n $ip ]] || return 1

  cc=$(tor_control "GETINFO ip-to-country/$ip" 2>/dev/null \
    | sed -n 's/^250[-+]ip-to-country\/[^=]*=//p' | head -1 | tr -d '\r')
  [[ $cc == "??" ]] && cc=""

  age=0
  if [[ -n $created ]]; then
    now=$(date -u +%s)
    # TIME_CREATED is UTC ISO 8601 with microseconds; date handles it once the
    # fractional part is trimmed and the zone is marked explicitly.
    local created_ts
    created_ts=$(date -u -d "${created%%.*}Z" +%s 2>/dev/null) || created_ts=""
    [[ -n $created_ts ]] && age=$(( now - created_ts ))
    (( age < 0 )) && age=0
  fi

  printf '%s\t%s\t%s' "$cc" "$ip" "$age"
}
