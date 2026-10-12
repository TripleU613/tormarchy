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

# The -- matters: a pinned string can itself start with a dash, a command-line
# flag for instance, and grep would otherwise take it for one of its own.
want() {          # want <description> <file> <fixed-string>
  if grep -Fq -- "$3" "$2"; then ok "$1"; else bad "$1 (missing in $2: $3)"; fi
}
reject() {        # reject <description> <file> <fixed-string>
  if grep -Fq -- "$3" "$2"; then bad "$1 (found in $2: $3)"; else ok "$1"; fi
}
# For order, which a single fixed string cannot pin: grep -F given two lines
# matches either of them on its own.
want_next() {     # want_next <description> <file> <line> <the line after it>
  if awk -v a="$3" -v b="$4" 'hit && index($0, b) { found = 1; exit } { hit = index($0, a) > 0 } END { exit !found }' "$2"
  then ok "$1"; else bad "$1 (in $2, '$4' does not follow '$3')"; fi
}

echo "Privilege boundary"

# setup and uninstall must never be reachable through the polkit grant. The
# single action pins the program path and not its arguments, so a passwordless
# grant for connecting would otherwise also cover uninstalling.
want "setup requires a terminal" tormarchy "require_terminal_root"
want "the terminal check refuses pkexec" tormarchy 'PKEXEC_UID:-} ]] || die'

# The passwordless grant reaches only the subcommands that need root, never all
# of them. One action per subcommand, pinned to it as the first argument; pkexec
# skips an action whose argv1 does not match, and with no match at all falls
# back to its generic action, which asks for a password and which the rule never
# grants. An action pinned to the path with no argv1 matches every subcommand:
# that was the whole hole, browser-exec handing out root shells.
if python3 - <<'PY'
import re, sys
policy = open("etc/polkit/com.tormarchy.policy").read()
script = open("tormarchy").read()
allowed = re.search(r"^PKEXEC_COMMANDS=\(([^)]*)\)", script, re.M).group(1).split()
actions = re.findall(r'<action id="([^"]+)">(.*?)</action>', policy, re.S)
problems = []
argv1s = []
for action_id, body in actions:
    path = re.search(r'exec\.path">([^<]*)<', body)
    argv1 = re.search(r'exec\.argv1">([^<]*)<', body)
    if not path or path.group(1) != "/usr/local/bin/tormarchy":
        problems.append(f"{action_id}: not pinned to /usr/local/bin/tormarchy")
    if not argv1:
        problems.append(f"{action_id}: no exec.argv1, so it grants every subcommand")
    else:
        argv1s.append(argv1.group(1))
if sorted(argv1s) != sorted(allowed):
    problems.append(f"policy grants {sorted(argv1s)}, PKEXEC_COMMANDS lists {sorted(allowed)}")
for forbidden in ("browser-exec", "browser", "setup", "uninstall", "boot", "menu"):
    if forbidden in argv1s or forbidden in allowed:
        problems.append(f"{forbidden} is reachable as root through polkit")
if problems:
    print("\n".join("    " + p for p in problems))
    sys.exit(1)
PY
then ok "polkit grants exactly the subcommands that need root, each pinned by its first argument"
else bad "the polkit grant reaches more than it should (above)"; fi
want "the dispatcher refuses anything else under pkexec" tormarchy 'if [[ -n ${PKEXEC_UID:-} && " ${PKEXEC_COMMANDS[*]} " != *" $command "* ]]; then'
want "browser-exec never runs as root" tormarchy '(( EUID != 0 )) || die "browser-exec never runs as root"'
want "nor does a browser launch" tormarchy '(( EUID != 0 )) || die "launch the browser as yourself, not as root"'

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

# Switching on moves the machine only onto a route already proven to carry
# traffic. Without it the switch lands on circuits still being built, and
# everything on the machine waits on them: the big drop.
if awk '/^cmd_connect\(\) \{/ { inside = 1 } inside && /find_route \|\|/ { found = NR } inside && /apply_rules "\$mode"/ { applied = NR } inside && /^}/ { exit } END { exit !(found && applied && found < applied) }' tormarchy
then ok "connect proves a route before any rule goes on"; else bad "connect applies rules without proving a route first"; fi

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
want "latency streaming is gated on the panel being open" Service.qml "root.watching && root.circuitReady"
want "the panel sets watching from opened" Panel.qml "tor.watching = opened"

# ...and on the connection being on, not merely on the daemon running. Gating the
# stream on raw torRunning left a live millisecond reading ticking under an OFF
# switch, because the daemon outlives the disconnect click. Requiring the
# connection to be real and not just asked for keeps it from pointing a stream of
# failing requests at a proxy that cannot carry them during a long bootstrap.
want "latency stops when the toggle is off, not just when tor stops" Service.qml "readonly property bool circuitReady: torRunning && active"
want "latency waits for a real connection, not an intended one" Service.qml '(effectiveMode === "socks" || connected)'

# "On" is mode-dependent. Browser-only installs no rules, so measuring it by
# `connected` left the switch stuck on OFF with tor plainly running, and made
# clicking it connect again rather than disconnect -- the mode could not be
# turned off from the panel at all.
want "the on-state is mode-aware" Service.qml 'effectiveMode === "socks" ? torRunning : connected'
want "the settle match uses the same definition" Service.qml "liveConnection === (_desired === 1)"

# A status read is several control-port round trips and outlives a fast action,
# so a reply issued before it repainted the panel with a live circuit under an
# OFF switch.
want "stale status reads are dropped" Service.qml "root._statusEpoch !== root._actionEpoch"

# The toggle's busy state is action-only. Folding the routine status poll in made
# the switch flash and refuse clicks on every refresh.
want "busy tracks actions, not the status poll" Service.qml "readonly property bool busy: (!!actionProcess && actionProcess.running)"

# A connect waits up to two minutes for bootstrap, and the switch used to refuse
# every click for all of it. Switching off during that wait cancels instead.
want "a pending connect can be cancelled from the switch" Service.qml "readonly property bool canToggle"
want "the panel gates the switch on canToggle, not busy" Panel.qml "tor.installed && tor.canToggle"
want "connect stops waiting when tor goes away" tormarchy "tor_is_running || break"
want_next "connect re-checks tor under the lock before applying" tormarchy \
  "cancelled_unless_running" 'apply_rules "$mode" || die'


# The live stream keeps one connection open, and a connection stays on the
# circuit it was opened on. Without a restart, "New circuit" left the gauge
# timing the circuit that had just been abandoned.
want "the latency stream restarts across a new circuit" Service.qml "!root._pingHeld"

echo
echo "Loading in the shell"

# Qt 6.12 ships a Color type of its own. A bare `Color.x` resolves to it, comes
# back undefined, and a colour given undefined falls back to black -- the
# switch's "on" track went black on a dark bar. Qualified everywhere.
bare=$(awk '
  { line = $0; sub(/\/\/.*/, "", line) }
  line ~ /(^|[^.A-Za-z_])Color\.[a-z]/ { print FILENAME ":" FNR ": " $0 }
' ./*.qml)
if [[ -z $bare ]]; then ok "no bare Color. in any QML file"; else bad "bare Color. (use Commons.Color):"; printf '        %s\n' "$bare"; fi

# Quickshell's IpcHandler, never Omarchy's ShellIpc. ShellIpc is newer than the
# latest Omarchy release, and a widget naming a type its shell lacks is left out
# of the bar without a word -- which is what 1.2.0 did on every released shell.
want "the panel's IPC is a plain IpcHandler" Panel.qml "IpcHandler {"
reject "no ShellIpc, which released Omarchy does not have" Panel.qml "ShellIpc {"

# The bar mark centres every layer on the box it is given. Omarchy 4.0.x
# stretches a custom icon over its whole canvas, and layers placed against
# their own size drifted into the top-left corner of the stretched box.
reject "no layer of the mark is placed against its own size" TorIcon.qml "(root.size - width) / 2"
want "the size falls back where the shell has no bar icon size" Panel.qml "Style.bar.iconSize || "

# A third-party plugin's shell facade only summons the plugin's own id. The
# speed test summon was always refused, after closing the panel first.
reject "no summon of another plugin's panel" Panel.qml 'summon("omarchy.'

echo
echo "Escapes and disconnections"

# IPAddressDeny on a --user scope is a BPF program a user manager cannot load.
# systemd logs it and carries on without it, so the "locked to Tor" browser was
# not locked at all. The lock is an nftables rule on the scope's cgroup instead.
reject "browser-only does not rely on IPAddressDeny" tormarchy "--property=IPAddressDeny"
want "browser-only is locked by cgroup in nftables" tormarchy 'socket cgroupv2 level'
want "the browser is not started if the lock fails" tormarchy 'could not lock the browser to Tor, so it was not started'
# A running Chromium handed a new window opens it in the existing, unproxied
# process. Its own profile forces an instance of its own.
want "chromium gets an instance of its own" tormarchy '--user-data-dir=$data/chromium-$name'
want "qutebrowser gets an instance of its own" tormarchy '--basedir "$data/qutebrowser"'

# The boot marker held nothing, so boot reconnected in lan whatever the user had
# chosen: Maximum came back as Standard after every reboot.
want "the boot marker records the mode" tormarchy 'remember_connected "$mode"'
want "connect falls back to the remembered mode" tormarchy 'mode=$(remembered_mode)'

# The bridge fallback wrote bridge lines and never restarted tor, so it could
# not have worked.
want_next "the bridge fallback restarts tor onto the bridges" tormarchy \
  'bridge_activate "$BRIDGE_STASH" >/dev/null 2>&1; then' "bridge_reload"

# The published cookie copy was preferred by the very lookup meant to refresh it,
# so after any tor restart the control port stayed dark.
want "publishing reads tor's own cookie" tormarchy 'for candidate in "${TOR_COOKIES[@]}"; do'

# Loopback v6 stays up, and the pre-connect state is saved once only.
want "loopback IPv6 stays up while connected" tormarchy "printf '0' >\"\$IPV6_CONF/lo/disable_ipv6\""
want "the IPv6 state is saved once per connection" tormarchy 'if [[ ! -e $IPV6_STATE_FILE ]]; then'

echo
echo "Panel tools"

# Omarchy's rule is its own sender, never raw notify-send: the sender is what
# applies the user's notification settings, do-not-disturb included.
want "toasts go through omarchy-notification-send" Model.js '["omarchy-notification-send",'
for f in Panel.qml Service.qml Model.js; do
  reject "no raw notify-send in $f" "$f" '"notify-send"'
done

# The leak check makes real requests -- check.torproject.org, a direct IPv6
# probe -- so it runs when asked and never on a schedule, like the latency
# gauge's rule. A Timer that so much as mentions it is the start of a beacon.
if python3 - <<'PY'
import re, sys
for name in ("Panel.qml", "Service.qml"):
    src = open(name).read()
    for m in re.finditer(r"\bTimer\s*\{", src):
        depth, i = 0, m.end() - 1
        while True:
            depth += {"{": 1, "}": -1}.get(src[i], 0)
            if depth == 0:
                break
            i += 1
        if re.search(r"checkLeaks|doctorProcess|doctor", src[m.start():i]):
            sys.exit(1)
PY
then ok "no Timer runs the leak check"; else bad "a Timer runs the leak check"; fi

# Bridge lines are what a bridge exists to hide. argv is world-readable in
# /proc, so they go to tormarchy over stdin and nowhere else: never a command
# line, never a file the root helper is pointed at.
want "bridge check takes no arguments" Service.qml '["bash", "-c", "exec tormarchy bridge check"]'
want "bridge set takes no arguments" Service.qml '["bash", "-c", "exec tormarchy bridge set"]'
want_next "bridge set's stdin is closed after the lines, which is its end of input" Service.qml \
  'bridgeSetProcess.write(root._pendingBridgeText' 'bridgeSetProcess.stdinEnabled = false'
want_next "so is bridge check's" Service.qml \
  'bridgeCheckProcess.write(root._pendingBridgeText' 'bridgeCheckProcess.stdinEnabled = false'
if python3 - <<'PY'
import re, sys
src = open("Service.qml").read()
uses = [l.strip() for l in src.splitlines() if "_pendingBridgeText" in l]
allowed = re.compile(r"^(property string _pendingBridgeText: \"\"|_pendingBridgeText = \"\"|root\._pendingBridgeText = (\"\"|text)"
                     r"|if \(bridgeWorking \|\| busy \|\| _pendingBridgeText === \"\"\) return"
                     r"|bridge(Set|Check)Process\.write\(root\._pendingBridgeText \+ \"\\n\"\))$")
bad = [u for u in uses if not allowed.match(u)]
if bad:
    print("\n".join("    " + b for b in bad))
    sys.exit(1)
PY
then ok "the pasted text is only stored, cleared, and written to stdin"
else bad "the pasted text is used somewhere other than stdin (see above)"; fi
reject "the root helper takes no bridge file argument" tormarchy 'bridge set <file'
want "the panel-readable summary is counts only" tormarchy 'bridge_counts "$BRIDGE_STASH" >"$staging"'

echo
echo "The Omarchy menu"

# The menu file is the user's, so it is written as the user: root writing into a
# home directory follows whatever symlinks are there. sudo tormarchy menu add
# goes to the person behind sudo, never to root's own home.
want "menu edits run as the user, through runuser" tormarchy 'runuser -u "$user" -- env HOME="$home" "$SELF" menu "$verb"'
want "and refuse to run as root for themselves" tormarchy 'die "run this as yourself, not as root"'
# A file that does not parse already costs its owner every entry; one this made
# unparseable would be worse. Checked before and after, and left alone if not.
want "a file that does not parse is left alone" tormarchy 'does not parse as it is; fix it first, nothing was changed'
want "and nothing is written that would not parse" tormarchy 'the menu would not parse after the change, so nothing was changed'
want "uninstall takes the rows out" tormarchy 'menu_as_user "$SUDO_USER" remove'

# The marketplace's security scan reads a script that downloads (curl) and also
# runs an interpreter as one that may run what it fetched, and blocks it. A perl
# one-liner to parse the menu file was enough to turn the verdict to
# needs-fixes; sed -z does the same job without being an interpreter.
for interpreter in perl python3 ruby node; do
  if grep -qE "(^|[[:space:];|&(])$interpreter[[:space:]]" tormarchy; then
    bad "tormarchy runs $interpreter, which the marketplace scan treats as running a download"
  else
    ok "tormarchy runs no $interpreter"
  fi
done

echo
echo "Panel copy"

# Labels on the panel, never sentences: anything that explains or instructs is
# a hover bubble. So a string longer than a label may only appear where it is
# given to a tooltip.
if python3 - <<'PY'
import re, sys
bad = []
for n, line in enumerate(open("Panel.qml"), 1):
    for text in re.findall(r'qsTr\("((?:[^"\\]|\\.)*)"\)', line):
        if len(text) > 32 and not re.search(r"\b(tip|tooltipText)\s*:|wrapTip\(", line):
            bad.append(f"    Panel.qml:{n}: {text}")
if bad:
    print("\n".join(bad))
    sys.exit(1)
PY
then ok "no string on the panel is longer than a label, unless it is a bubble's"
else bad "a sentence is on the panel itself (above); put it in a tooltip"; fi
want "messages from the helper are shown cut to one line" Panel.qml "component Brief: Text {"
want "and wrapped for their bubble, which draws one line however long" Panel.qml "text: Model.wrapTip(brief.full)"

echo
if (( failures == 0 )); then
  echo "  all decisions still hold"
else
  echo "  $failures decision(s) changed -- if that was deliberate, update this file"
  exit 1
fi
