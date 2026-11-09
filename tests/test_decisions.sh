#!/usr/bin/env bash
#
# Pins decisions that are easy to undo by accident.
#
# The behaviour is tested where it lives: the Rust crates' own tests, the
# daemon scenarios, the QML load and wiring tests. What is pinned here is what
# spans files -- the polkit policy against the helper's allow-list, the fixed
# paths the bootstrap and the panel agree on -- and what only reads as source:
# that the QML stays drawing-only.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.." || exit 1

failures=0
ok()   { printf '  \033[32mok\033[0m    %s\n' "$1"; }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; failures=$(( failures + 1 )); }

want() {          # want <description> <file> <fixed-string>
  if grep -Fq -- "$3" "$2"; then ok "$1"; else bad "$1 (missing in $2: $3)"; fi
}
reject() {        # reject <description> <file> <fixed-string>
  if grep -Fq -- "$3" "$2"; then bad "$1 (found in $2: $3)"; else ok "$1"; fi
}

HELPER=crates/tormarchy/src/main.rs
INSTALL=crates/tormarchy/src/install.rs
BROWSER=crates/tormarchy/src/browser.rs

echo "Marketplace"

# The marketplace's security scan fails closed on any binary file whose name
# contains install, installer, setup or uninstall: it cannot read inside it, so
# it will not vouch for it, and the listing cannot be verified at all.
named=$(git ls-files | grep -iE '(install|setup)[^/]*$' | while read -r f; do
  [[ -f $f ]] && ! grep -qI . "$f" && echo "$f"
done)
if [[ -z $named ]]; then ok "no binary file is named like an installer"; else bad "binary named like an installer, which the marketplace scan refuses: $named"; fi

echo "Privilege boundary"

# The passwordless grant reaches only the subcommands that need root. One action
# per subcommand, pinned to the installed path and to the subcommand as its
# first argument; an action with no argv1 matches every subcommand, which once
# handed out root shells through browser-exec.
if python3 - <<'PY'
import re, sys
policy = open("etc/polkit/com.tormarchy.policy").read()
src = open("crates/tormarchy/src/main.rs").read()
allowed = re.findall(r'"([a-z-]+)"', re.search(r"const PKEXEC_COMMANDS: \[&str; \d+\] = \[([^\]]*)\]", src).group(1))
actions = re.findall(r'<action id="([^"]+)">(.*?)</action>', policy, re.S)
problems, argv1s = [], []
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
for forbidden in ("browser-exec", "browser", "setup", "uninstall", "boot", "menu", "doctor", "status"):
    if forbidden in argv1s or forbidden in allowed:
        problems.append(f"{forbidden} is reachable as root through polkit")
if problems:
    print("\n".join("    " + p for p in problems))
    sys.exit(1)
PY
then ok "polkit grants exactly the subcommands that need root, each pinned by its first argument"
else bad "the polkit grant reaches more than it should (above)"; fi

want "the dispatcher refuses anything else under pkexec" "$HELPER" '!PKEXEC_COMMANDS.contains(&command.as_str())'
want "setup and uninstall require a terminal and sudo" "$HELPER" 'fn require_terminal_root()'
want "browser-exec never runs as root" "$BROWSER" '"browser-exec never runs as root"'
want "nor does a browser launch" "$BROWSER" '"launch the browser as yourself, not as root"'
want "the helper fixes PATH before anything runs" "$HELPER" 'std::env::set_var("PATH", sys::FIXED_PATH);'
want "and closes its umask" "$HELPER" 'libc::umask(0o077);'
want "escalation re-runs this exact binary" "$HELPER" 'Command::new(runner).arg(self_path()).args(argv).exec()'

# The daemon runs as the user and keeps the user's PATH -- Omarchy's notifier
# lives only there -- so it must never be the thing that runs as root.
reject "the daemon never escalates" crates/tormarchyd/src/daemon.rs 'pkexec'
reject "nor uses sudo" crates/tormarchyd/src/daemon.rs '"sudo"'
reject "the root helper has no test seam" "$HELPER" 'TORMARCHY_ROOT'
reject "nor does the core it runs" crates/tormarchy-core/src/paths.rs 'TORMARCHY_ROOT'

# A double hyphen inside an XML comment makes the whole document unparseable,
# polkitd registers nothing, and pkexec prompts with no explanation anywhere.
if python3 - <<'PY'
import re, sys
s = open("etc/polkit/com.tormarchy.policy").read()
sys.exit(any("--" in c[4:-3] for c in re.findall(r"<!--.*?-->", s, re.S)))
PY
then ok "no double hyphen inside an XML comment"
else bad "a double hyphen appears inside an XML comment"
fi

echo
echo "What lands as root"

# Nothing installed as root is read from the user-writable plugin checkout: the
# files are inside the binary the bootstrap verified.
for f in etc/torrc.d/00-tormarchy.conf etc/tmpfiles/tor.conf etc/polkit/com.tormarchy.policy etc/polkit/49-tormarchy.rules.in; do
  want "$f is compiled into the helper" "$INSTALL" "include_str!(\"../../../$f\")"
done
want "setup verifies the release's Sigstore signature" setup 'cosign verify-blob'
want "against this repository's release workflow" setup '--certificate-identity "https://github.com/$REPO/$WORKFLOW"'
want "at the commit the tag names on GitHub" setup '--certificate-github-workflow-sha "$commit"'
want "found on GitHub, not in this checkout" setup 'git ls-remote "https://github.com/$REPO.git"'
want "extracting only the two binaries" setup 'tormarchy tormarchyd'
want "and checking the binary is the version the plugin declares" setup '[[ $got == "tormarchy $version" ]]'
reject "uninstall does not disable a tor it never enabled" "$INSTALL" '"disable", "tor.service"'
reject "setup does not enable tor at boot" "$INSTALL" '"enable", "tor.service"'
reject "the ruleset is never persisted" crates/tormarchy-core/src/firewall.rs "/etc/nftables.conf"
reject "IPv6 is never persisted to sysctl.d" crates/tormarchy-core/src/firewall.rs "/etc/sysctl.d"

echo
echo "The panel draws; the daemon decides"

[[ ! -e Model.js ]] && ok "no Model.js" || bad "Model.js is back; that logic lives in tormarchy-core::view"
for f in Panel.qml Service.qml TorIcon.qml SpeedGauge.qml TorWordmark.qml; do
  reject "$f runs no shell" "$f" '"bash"'
  reject "$f calls no notify-send" "$f" 'notify-send'
done
want "the service runs the installed daemon by its fixed path" Service.qml 'command: ["/usr/local/bin/tormarchyd"]'
reject "and starts nothing else" Service.qml '"tormarchy"'
reject "the panel starts no process of its own" Panel.qml 'Process {'
want "the panel sets watching from opened" Panel.qml "tor.watch(opened)"
want "the panel's IPC is a plain IpcHandler" Panel.qml "IpcHandler {"
reject "no ShellIpc, which released Omarchy does not have" Panel.qml "ShellIpc {"
reject "no summon of another plugin's panel" Panel.qml 'summon("omarchy.'
# Setup asks the password in Omarchy's own dialog: pkexec of the plugin's
# bootstrap, which polkit only reaches through its always-ask default action.
# Never the installed helper, whose runtime commands the passwordless grant
# covers, and the bootstrap hands the helper sudo's SUDO_USER, not PKEXEC_UID.
want "setup is asked for through pkexec of the plugin's own bootstrap" Service.qml 'command: ["pkexec", root.pluginDir + "/setup"]'
reject "never of the installed helper" Service.qml '"pkexec", "/usr/local/bin/tormarchy"'
want "the bootstrap hands the helper SUDO_USER, never PKEXEC_UID" setup 'unset PKEXEC_UID'
want "a terminal and sudo where there is no polkit agent" Panel.qml '"sudo " + pluginDir + "/setup"'
reject "no layer of the mark is placed against its own size" TorIcon.qml "(root.size - width) / 2"
want "the size falls back where the shell has no bar icon size" Panel.qml "Style.bar.iconSize || "

# With `omarchy toggle animations` off the shell sets Style.reduceMotion, and a
# looping animation must stop rather than spin at zero duration. Both tokens are
# newer than Omarchy 4.0.x, so every read of them is guarded.
want "the bar pulse stops under reduced motion" TorIcon.qml "root.pulsing && root.coreVisible && !root.reduceMotion"
want "and connecting still reads as a half-lit core" TorIcon.qml "root.reduceMotion ? 0.5 : pulse.level"
want "the gauge's spinner stops under reduced motion" SpeedGauge.qml "running: root.busy && !root.reduceMotion"
for f in TorIcon.qml SpeedGauge.qml Panel.qml; do
  want "$f reads Style.duration only where the shell has it" "$f" 'typeof Style.duration === "function"'
done

# Every Text that shows something from outside carries PlainText.
if python3 - <<'PY'
import re, sys
bad = []
for name in ("Panel.qml",):
    src = open(name).read()
    for m in re.finditer(r"\bText\s*\{", src):
        depth, i = 0, m.end() - 1
        while True:
            depth += {"{": 1, "}": -1}.get(src[i], 0)
            if depth == 0:
                break
            i += 1
        body = src[m.start():i]
        own = re.sub(r"\{[^{}]*\{.*", "", body, flags=re.S)
        if "textFormat: Text.PlainText" not in body:
            bad.append(f"    {name}:{src[:m.start()].count(chr(10)) + 1}")
if bad:
    print("\n".join(bad))
    sys.exit(1)
PY
then ok "every Text on the panel is plain text"
else bad "a Text could render markup from outside (above)"; fi

# Labels on the panel, never sentences: anything longer is a bubble's.
if python3 - <<'PY'
import re, sys
bad = []
for n, line in enumerate(open("Panel.qml"), 1):
    for text in re.findall(r'text:\s*"((?:[^"\\]|\\.)*)"', line):
        if len(text) > 32:
            bad.append(f"    Panel.qml:{n}: {text}")
if bad:
    print("\n".join(bad))
    sys.exit(1)
PY
then ok "no string on the panel is longer than a label"
else bad "a sentence is on the panel itself (above); put it in a tooltip"; fi

# The leak check makes real requests, so it runs when asked and never on a
# schedule: a Timer that so much as mentions it is the start of a beacon.
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
        if re.search(r"checkLeaks|measure|doctor", src[m.start():i]):
            sys.exit(1)
PY
then ok "no Timer runs the leak check or a measurement"; else bad "a Timer runs the leak check or a measurement"; fi

echo
echo "Release"

# Every action a workflow runs is pinned to a full commit.
if grep -hE '^\s*-?\s*uses:' .github/workflows/*.yml | grep -vqE '@[0-9a-f]{40}( |$)'; then
  bad "an action is not pinned to a full commit: $(grep -hE '^\s*-?\s*uses:' .github/workflows/*.yml | grep -vE '@[0-9a-f]{40}( |$)' | head -1)"
else
  ok "every action is pinned to a full commit"
fi
want "the release is signed keyless by its own workflow" .github/workflows/auto-release.yml 'cosign sign-blob --yes --bundle'
want "with an OIDC identity to sign with" .github/workflows/auto-release.yml 'id-token: write'
want "and tagged at the commit it was built from" .github/workflows/auto-release.yml 'git tag -a "v$VERSION" -m "v$VERSION" "${{ github.sha }}"'

echo
if (( failures == 0 )); then
  echo "  all decisions still hold"
else
  echo "  $failures decision(s) changed -- if that was deliberate, update this file"
  exit 1
fi
