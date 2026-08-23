#!/usr/bin/env bash
#
# Pins decisions that are easy to undo by accident.
#
# Quickshell's Process, Panel and the qs.Ui kit only exist inside a running
# Omarchy shell, and the privileged half only does anything as root on a machine
# with tor installed. So the behaviour that matters cannot be exercised here.
# What can be pinned is the source: each check below stands for a decision that
# cost something to learn, with a comment saying what breaks without it.
#
# Idea borrowed from huacnlee/omarchy-mihoro's test_panel_source.sh.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

failures=0
ok()   { printf '  \033[32mok\033[0m    %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; failures=$(( failures + 1 )); }

want() {          # want <description> <file> <fixed-string>
  if grep -Fq "$3" "$2"; then ok "$1"; else bad "$1 (missing in $2: $3)"; fi
}
reject() {        # reject <description> <file> <fixed-string>
  if grep -Fq "$3" "$2"; then bad "$1 (found in $2: $3)"; else ok "$1"; fi
}

echo "Privilege boundary"

# setup and uninstall must never be reachable through the polkit grant. The
# single action pins the program path and not its arguments, so a passwordless
# grant for connecting would otherwise also cover uninstalling.
want "setup requires a terminal" tormarchy "require_terminal_root"
want "the terminal check refuses pkexec" tormarchy 'PKEXEC_UID:-} ]] || die'

# One action, one program path. More actions bought nothing and were five more
# places to get an XML comment wrong.
want "exactly one polkit action" etc/polkit/com.tormarchy.policy 'id="com.tormarchy.manage"'

# A double hyphen inside an XML comment makes the whole document unparseable,
# polkitd registers nothing, and pkexec falls back to prompting with no
# explanation anywhere. This cost three rounds to find once.
if python3 - <<'PY'
import re, sys
s = open("etc/polkit/com.tormarchy.policy").read()
sys.exit(any("--" in c[4:-3] for c in re.findall(r"<!--.*?-->", s, re.S)))
PY
then ok "no double hyphen inside an XML comment"
else bad "a double hyphen appears inside an XML comment"
fi

echo
echo "Bridges"

# A root-run helper must not read a path an unprivileged caller chose. The
# earlier version did, and also deleted that path afterwards behind a glob that
# matched traversal, giving arbitrary file read and delete as root.
# The escalation must come before the read, so the check is that require_root
# appears inside the set case rather than anywhere in the file.
if awk '/^  set\)/{f=1} f&&/require_root/{print;exit}' tormarchy | grep -Fq require_root; then
  ok "bridge set escalates before reading"
else
  bad "bridge set does not escalate before reading stdin"
fi
reject "bridge set takes no caller path" tormarchy '/tmp/.tormarchy-bridges'

echo
echo "Ordering in the ruleset"

# Order is the whole correctness argument for the firewall, and every one of
# these was a real leak.
want "onion range is dropped in the filter chain" tormarchy 'ip daddr $VIRT_NET drop'
want "forward traffic is dropped" tormarchy "hook forward priority filter; policy drop"
want "invalid conntrack state is dropped" tormarchy "ct state invalid drop"
want "IPv6 is dropped explicitly" tormarchy "meta nfproto ipv6 drop"

# Rules go on only after a full bootstrap. Applying them earlier redirects
# everything into a daemon that cannot carry it, which looks exactly like a dead
# network.
want "connect waits for bootstrap before applying" tormarchy 'pct=$(wait_bootstrap 120)'
want "a stall applies nothing" tormarchy "No rules applied, networking is unchanged"

# The kernel IPv6 switch is written to /proc, never /etc/sysctl.d. Persisting it
# means a crash leaves IPv6 dead after the next reboot with nothing explaining
# why.
want "IPv6 is disabled through /proc" tormarchy "/proc/sys/net/ipv6/conf"
reject "IPv6 is not persisted to sysctl.d" tormarchy "sysctl.d/98"

echo
echo "Recovery"

# Nothing is written to /etc/nftables.conf, so a reboot always restores normal
# networking. That is the guaranteed way out of a wedged ruleset.
reject "the ruleset is never persisted" tormarchy "/etc/nftables.conf"

# The boot receiver is the one exception, and it must stay opt-in.
want "boot reconnect is gated on a marker" etc/systemd/tormarchy-boot.service "ConditionPathExists"
want "uninstall removes the boot unit" tormarchy 'rm -f "$BOOT_UNIT"'

# Removing the rules comes before removing the tool that knows how to remove
# them.
want "uninstall drops the firewall first" tormarchy "Removing any firewall rules"

echo
echo "Runtime state"

# The circuit cache lived at ${XDG_RUNTIME_DIR:-/tmp}/tormarchy-path.cache and
# was read and then truncated through that path directly. In the /tmp fallback
# the name is globally predictable, so a symlink planted there turned every
# panel refresh into a write into another file the user could reach. Found in
# marketplace security review at ed6795e.
reject "no /tmp fallback for runtime state" tormarchy 'XDG_RUNTIME_DIR:-/tmp'
reject "no globally predictable cache name" tormarchy "tormarchy-path.cache"
want "runtime state requires a private directory" tormarchy "runtime_dir()"
want "the runtime directory must be owned and not a symlink" tormarchy '[[ -n $base && ! -L $base && -d $base && -O $base ]] || return 1'
want "the runtime directory is created 0700" tormarchy 'mkdir -m 0700 -- "$dir"'

# An entry is read through the flags of a single open, not through a stat of the
# name first. Testing the name and then opening it are two steps, and a process
# running as this same user -- the only account that can reach inside a 0700
# directory -- can swap the entry for a symlink or a fifo in between, so the
# earlier type and owner tests said nothing about what was actually opened.
# Ownership is not the boundary within one uid anyway.
reject "the cache read does not stat the name before opening it" tormarchy '[[ ! -L $path && -f $path'
want "the cache read is one no-follow, nonblocking open" tormarchy "iflag=nofollow,nonblock"
want "the cache read is bounded by that same open" tormarchy 'bs="$CACHE_BYTE_LIMIT" count=1'

# Writing through the path itself is what made a planted symlink useful. A
# fresh mktemp file plus a rename cannot follow a link, and leaves no
# half-written line for a concurrent reader.
want "cache writes go through a fresh temporary file" tormarchy 'tmp=$(mktemp -- "$path.XXXXXX"'
want "cache writes land by atomic rename" tormarchy 'mv -f -- "$tmp" "$path"'

# Same class, worse blast radius: tor --verify-config runs as root during setup
# and its output went to a fixed /tmp name, so a symlink planted there was a
# root-owned truncate of any file on the system.
reject "setup does not write its verify log to a fixed path" tormarchy "/tmp/tormarchy-verify.log"
want "the verify log is created by mktemp" tormarchy 'verify_log=$(mktemp --'

echo
echo "Control port"

# Every caller captures a control-port reply whole in a shell variable, and the
# panel asks again every few seconds. Read with a bare cat, a faulty or
# compromised local endpoint that answers without ever stopping grew that
# process for as long as the timeout allowed -- 367 MB inside five seconds,
# measured against a flooding endpoint. The ceiling has to be applied while the
# socket is read, not after command substitution has already retained the reply.
reject "the control-port reply is not read without a ceiling" tormarchy "cat <&3"
want "the control-port reply has a byte ceiling" tormarchy 'head -c "$CONTROL_REPLY_BYTE_LIMIT" <&3'
want "the ceiling is applied inside the timeout" tormarchy 'timeout "${TOR_CONTROL_TIMEOUT:-5}" head -c'

echo
echo "The panel talks to the dispatcher"

# The fifteen separate executables are gone. Any leftover hyphenated call would
# be a command that no longer exists.
reject "no calls to the old hyphenated helpers" Service.qml "tormarchy-"
want "status is read as JSON" Service.qml "exec tormarchy status --json"
want "live latency streams from pingd" Service.qml "exec tormarchy pingd"

# Measuring only while the panel is open. A request to a fixed host at a fixed
# interval forever is a recognisable traffic pattern.
want "latency streaming is gated on the panel being open" Service.qml "root.watching && root.torRunning"
want "the panel sets watching from opened" Panel.qml "tor.watching = opened"

echo
if (( failures == 0 )); then
  echo "  all decisions still hold"
else
  echo "  $failures decision(s) changed -- if that was deliberate, update this file"
  exit 1
fi
