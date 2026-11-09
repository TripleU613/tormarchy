#!/bin/bash
#
# Every check CI runs, in one script you can run yourself:
#
#   .github/scripts/check.sh
#
# CI calls this same file, so a green run here means a green run there. It needs
# Rust and Python, not Omarchy or Quickshell. Under sudo it also validates the
# rulesets against the kernel and runs the kill switch in network namespaces.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 1

failures=0
pass() { printf '  \033[32mok\033[0m    %s\n' "$1"; }
fail() { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; failures=$(( failures + 1 )); }
note() { printf '  \033[90m--\033[0m    %s\n' "$1"; }
group() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# ----------------------------------------------------------------------- rust --

group "Rust"

if ! command -v cargo >/dev/null 2>&1; then
  fail "cargo is not installed: the helper and the daemon cannot be built or tested"
else
  cargo fmt --all --check >/dev/null 2>&1 && pass "formatted" || fail "not formatted (cargo fmt --all)"
  if cargo clippy --workspace --all-targets --locked --quiet -- -D warnings >/tmp/tormarchy-clippy.log 2>&1; then
    pass "clippy reports nothing"
  else
    tail -30 /tmp/tormarchy-clippy.log
    fail "clippy has findings"
  fi
  if cargo test --workspace --locked --quiet >/tmp/tormarchy-cargo-test.log 2>&1; then
    pass "$(grep -h -o 'ok\. [0-9]* passed' /tmp/tormarchy-cargo-test.log | awk '{n += $2} END {print n}') unit and socket tests pass"
  else
    grep -E 'FAILED|panicked|error' /tmp/tormarchy-cargo-test.log | head -20
    fail "cargo test failed"
  fi
  cargo build --locked --quiet 2>/dev/null && pass "the helper and the daemon build" || fail "cargo build failed"
fi
bin=target/debug/tormarchy

# The release is the manifest's version: setup fetches by it, and the panel
# offers "Update helper" when the two differ.
manifest_version=$(python3 -c 'import json; print(json.load(open("manifest.json"))["version"])')
cargo_version=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml)
[[ $manifest_version == "$cargo_version" ]] && pass "manifest and binaries are both $cargo_version" \
  || fail "manifest.json says $manifest_version but Cargo.toml says $cargo_version"

group "Bootstrap"

bash -n setup 2>/dev/null && pass "setup parses" || fail "setup has a syntax error"
[[ -x setup ]] && pass "setup is executable" || fail "setup is not executable (chmod +x)"
if command -v shellcheck >/dev/null 2>&1; then
  shellcheck --severity=warning setup tests/kernel/killswitch.sh .github/scripts/*.sh tests/run && pass "shellcheck reports nothing" \
    || fail "shellcheck has findings"
else
  note "shellcheck not installed, skipping lint"
fi
# What setup verifies is pinned to what the release job signs with.
grep -q 'WORKFLOW=".github/workflows/auto-release.yml@refs/heads/master"' setup \
  && grep -q 'auto-release.yml@refs/heads/master' .github/workflows/auto-release.yml \
  && pass "setup verifies the identity the release job signs with" \
  || fail "setup and the release workflow disagree on the signing identity"

# ----------------------------------------------------------------------- data --

group "Manifest"

python3 - <<'PY' || failures=$(( failures + 1 ))
import json, os, re, sys

fails = []
def bad(msg): fails.append(msg)

try:
    m = json.load(open("manifest.json"))
except Exception as e:
    print(f"  \033[31mFAIL\033[0m  manifest.json is not valid JSON: {e}")
    sys.exit(1)

# Mirrors the checks in omarchy-plugin-validate, so a plugin that passes here is
# one the running shell will actually load rather than silently skip.
if m.get("schemaVersion") != 1:
    bad("schemaVersion must be the number 1")

for field in ("id", "name", "version", "kinds", "entryPoints"):
    if not m.get(field):
        bad(f"missing required field '{field}'")

# The marketplace additionally wants these.
for field in ("author", "description", "license"):
    if not m.get(field):
        bad(f"missing '{field}' (required by the plugin marketplace)")

pid = m.get("id", "")
if not re.match(r"^[A-Za-z0-9][A-Za-z0-9._-]*$", pid):
    bad(f"invalid plugin id {pid!r}")
if pid.startswith("omarchy."):
    bad("plugin id uses the reserved omarchy.* namespace")

if len(str(m.get("version", ""))) > 64:
    bad("version is longer than 64 characters")

if not isinstance(m.get("kinds"), list) or not m["kinds"]:
    bad("kinds must be a non-empty array")

eps = m.get("entryPoints") or {}
if not isinstance(eps, dict):
    bad("entryPoints must be an object")
else:
    for key, path in eps.items():
        if not isinstance(path, str) or not path:
            bad(f"entry point {key!r} is empty")
        elif path.startswith("/") or ".." in path or "\n" in path:
            bad(f"entry point {key!r} is not a safe relative path: {path!r}")
        elif not os.path.isfile(path):
            bad(f"entry point {key!r} points at a missing file: {path}")

# A kind is a promise to supply something to load. Claiming one without its
# entry point installs and enables fine, then does nothing.
for kind, key in (("bar", "bar"), ("bar-widget", "barWidget"), ("menu", "menu"),
                  ("overlay", "overlay"), ("panel", "panel"), ("service", "service")):
    if kind in (m.get("kinds") or []) and key not in eps:
        bad(f"kind {kind!r} requires entryPoints.{key}")

section = ((m.get("barWidget") or {}).get("defaultSection"))
if section is not None and section not in ("left", "center", "right"):
    bad(f"barWidget.defaultSection must be left, center or right, not {section!r}")

if fails:
    for f in fails:
        print(f"  \033[31mFAIL\033[0m  {f}")
    sys.exit(1)

print(f"  \033[32mok\033[0m    manifest is valid ({pid} {m.get('version')}, kinds={m['kinds']})")
PY

# The shell refuses to load a plugin containing a symlink, because one could
# point a copied plugin back at any file on disk once it lands in the trusted
# plugins directory.
if link=$(find . -name .git -prune -o -type l -print -quit 2>/dev/null); [[ -z $link ]]; then
  pass "no symlinks in the tree"
else
  fail "symlinks are not allowed inside a plugin: $link"
fi

group "QML"

# qmllint cannot type-check these: they import qs.Commons and qs.Ui from
# Omarchy's shell, and Quickshell is not installable on a CI runner, so every
# symbol resolves to an error and drowns out anything real.
#
# What is checkable without that context is structure, and that is the failure
# mode that actually bites -- these files are edited by script, and an
# unbalanced brace turns into a widget that silently fails to load with the
# reason buried in the shell's log.
python3 - <<'PY' || failures=$(( failures + 1 ))
import glob, sys

def strip(src):
    """Remove strings and comments so only structural punctuation is counted."""
    out, i, n = [], 0, len(src)
    while i < n:
        c = src[i]
        if c in '"\'':
            quote, i = c, i + 1
            while i < n and src[i] != quote:
                i += 2 if src[i] == "\\" else 1
            i += 1
        elif src.startswith("/*", i):
            j = src.find("*/", i)
            i = n if j < 0 else j + 2
        elif src.startswith("//", i) and (i == 0 or src[i - 1] in " \t\n"):
            # Only a comment when it follows whitespace or a line start. A
            # regex literal like /^file:\/\// also contains "//", and treating
            # that as a comment eats the rest of the line -- which reported a
            # false unbalanced paren in Panel.qml.
            while i < n and src[i] != "\n":
                i += 1
        else:
            out.append(c)
            i += 1
    return "".join(out)

bad = []
for path in sorted(glob.glob("*.qml")):
    text = strip(open(path).read())
    # Braces and brackets only. Parentheses appear inside regex literals often
    # enough that counting them is more likely to report a phantom than a bug.
    for opener, closer, label in (("{", "}", "brace"), ("[", "]", "bracket")):
        d = text.count(opener) - text.count(closer)
        if d:
            bad.append(f"{path}: {abs(d)} unclosed {label}" if d > 0
                       else f"{path}: {abs(d)} extra closing {label}")

if bad:
    for b in bad:
        print(f"  \033[31mFAIL\033[0m  {b}")
    sys.exit(1)

print(f"  \033[32mok\033[0m    {len(glob.glob('*.qml'))} QML files are balanced")
PY

# Every file the manifest points at must be tracked, or a fresh clone gets a
# plugin that validates locally and fails for everyone else.
if command -v git >/dev/null 2>&1 && git rev-parse --git-dir >/dev/null 2>&1; then
  untracked=$(git ls-files --others --exclude-standard '*.qml' '*.js' 'manifest.json' 2>/dev/null)
  [[ -z $untracked ]] && pass "no untracked QML or JS" \
    || fail "untracked files a clone would not get: $(tr '\n' ' ' <<<"$untracked")"
fi

group "XML and assets"

if command -v xmllint >/dev/null 2>&1; then
  if xmllint --noout etc/polkit/com.tormarchy.policy 2>/dev/null; then
    pass "polkit policy is well-formed"
  else
    xmllint --noout etc/polkit/com.tormarchy.policy
    fail "polkit policy is not well-formed XML"
  fi
  xmllint --noout assets/logo.svg 2>/dev/null \
    && pass "logo.svg is well-formed" || fail "logo.svg is not well-formed"
else
  note "xmllint not installed, skipping XML checks"
fi

# The marketplace listing shows this. Not fatal, but a listing with no picture
# is a listing nobody clicks.
if [[ -f preview.png ]]; then
  pass "preview.png is present ($(du -h preview.png | cut -f1))"
else
  note "no preview.png -- the marketplace listing will have no image"
fi

# A double hyphen inside an XML comment does not merely break the comment, it
# makes the document unparseable. polkitd then registers no actions and pkexec
# silently falls back to its generic admin-auth action, so the only symptom is a
# password prompt no rule can suppress, with nothing logged to explain it. This
# has happened once; hence a check of its own.
if python3 - <<'PY'
import re, sys
s = open("etc/polkit/com.tormarchy.policy").read()
for c in re.findall(r"<!--.*?-->", s, re.S):
    if "--" in c[4:-3]:
        sys.exit(1)
PY
then
  pass "no double hyphen inside an XML comment"
else
  fail "a double hyphen appears inside an XML comment (this makes polkitd ignore the whole file)"
fi

# ------------------------------------------------------------------- ruleset --

group "Firewall ruleset"

# These assertions are regression tests for leaks that were actually shipped,
# not hypotheticals. Each one names the bug it guards against.
# Judged on output, not exit code. Run as root, --dry-run also validates against
# the kernel and exits non-zero when that is unavailable (a CI runner has no
# netlink access), even though the ruleset printed perfectly well. Generation and
# validation are separate questions, so they get separate checks.
for mode in strict lan socks; do
  "$bin" connect --dry-run "$mode" >"/tmp/ruleset-$mode.nft" 2>/dev/null || true
  if grep -q "table inet tormarchy" "/tmp/ruleset-$mode.nft" 2>/dev/null; then
    pass "$mode ruleset generates"
  else
    fail "the $mode ruleset came out empty or malformed"
  fi
done

ruleset=/tmp/ruleset-lan.nft

# The .onion virtual range 10.192.0.0/10 sits inside 10.0.0.0/8, so in lan mode
# a LAN accept placed first sends UDP aimed at a resolved .onion out in
# cleartext. It shipped that way once.
virt_line=$(grep -n "ip daddr 10.192.0.0/10 drop" "$ruleset" | head -1 | cut -d: -f1)
lan_line=$(grep -n "ip daddr { 10.0.0.0/8" "$ruleset" | tail -1 | cut -d: -f1)
if [[ -n $virt_line && -n $lan_line ]] && (( virt_line < lan_line )); then
  pass "onion range is dropped before the LAN exemption"
else
  fail "the 10.192.0.0/10 drop must come BEFORE the LAN accept, or .onion UDP leaks"
fi

# The usual resolver on a home network is the router, which the LAN rule would
# return -- sending every lookup out in cleartext while the onion stayed lit.
dns_line=$(grep -n "th dport 53 redirect" "$ruleset" | head -1 | cut -d: -f1)
lan_return=$(grep -n "ip daddr { 10.0.0.0/8" "$ruleset" | head -1 | cut -d: -f1)
if [[ -n $dns_line && -n $lan_return ]] && (( dns_line < lan_return )); then
  pass "DNS is redirected before the LAN exemption"
else
  fail "the DNS redirect must come BEFORE the LAN return, or DNS leaks to the router"
fi

# DNS over TLS can't be redirected into Tor's resolver, and a Pi-hole on the LAN
# offering it was accepted by the LAN rule: every name on the wire.
dot_line=$(grep -n "th dport 853 drop" "$ruleset" | head -1 | cut -d: -f1)
lan_accept=$(grep -n "ip daddr { 10.0.0.0/8" "$ruleset" | sed -n 2p | cut -d: -f1)
if [[ -n $dot_line && -n $lan_accept ]] && (( dot_line < lan_accept )); then
  pass "DNS over TLS is dropped before the LAN exemption"
else
  fail "the port 853 drop must come BEFORE the LAN accept, or DNS over TLS leaks to a LAN resolver"
fi

for mode in strict lan; do
  r="/tmp/ruleset-$mode.nft"
  # Forwarded traffic never passes the output hook, so a bridged VM, container
  # or hotspot leaves the NIC untouched without this chain.
  grep -q "hook forward" "$r" \
    && pass "$mode has a forward chain" \
    || fail "$mode has no forward chain: routed traffic bypasses every rule"

  grep -q "policy drop" "$r" \
    && pass "$mode defaults to drop" \
    || fail "$mode does not default to drop"

  grep -q "meta nfproto ipv6 drop" "$r" \
    && pass "$mode drops IPv6 explicitly" \
    || fail "$mode does not drop IPv6"

  grep -q "ct state invalid drop" "$r" \
    && pass "$mode drops invalid conntrack state" \
    || fail "$mode does not drop invalid state"
done

# nft can only validate against the kernel, so this is root-only and skipped
# elsewhere rather than faked.
if ! command -v nft >/dev/null 2>&1; then
  note "nft not installed, skipping ruleset validation"
elif (( EUID != 0 )); then
  note "nft check needs root, skipping (run: sudo .github/scripts/check.sh)"
elif ! nft list ruleset >/dev/null 2>&1; then
  # Probe first. A container or CI runner can have the nft binary with no
  # netlink access, and reporting that as "nft rejects the ruleset" would blame
  # the ruleset for the sandbox.
  note "nft cannot reach netlink here (container or restricted runner), skipping"
else
  for mode in strict lan; do
    if nft -c -f "/tmp/ruleset-$mode.nft" 2>/dev/null; then
      pass "nft accepts the $mode ruleset"
    else
      nft -c -f "/tmp/ruleset-$mode.nft" 2>&1 | head -3
      fail "nft rejects the $mode ruleset"
    fi
  done
fi

# The rules in a real kernel, between two network namespaces: nothing reaches
# the wire once they are on, and what was open before fails at once instead of
# hanging. Root and namespaces only; a restricted container skips it.
group "Kill switch, in a real kernel"
for mode in strict lan; do
  tests/kernel/killswitch.sh "/tmp/ruleset-$mode.nft" >"/tmp/killswitch-$mode.log" 2>&1
  case $? in
  0) pass "$mode: nothing leaves, and open connections fail at once" ;;
  2) note "$mode: needs root and network namespaces, skipping" ;;
  *) cat "/tmp/killswitch-$mode.log"; fail "$mode: the kill switch let something out, or left an app hanging" ;;
  esac
done

# --------------------------------------------------------------------- shape --

group "Name collisions"

# A property that shadows a base-type name either fails to load or, worse,
# silently stops the widget responding. Neither is caught until the shell tries
# to load the plugin.
python3 tests/test_qml_names.py || failures=$(( failures + 1 ))

group "Pinned decisions"

# Behaviour that cannot be exercised without a running shell and a root-owned
# firewall, pinned at the source level instead.
bash tests/test_decisions.sh || failures=$(( failures + 1 ))

group "The daemon"

# The real tormarchyd against a control port on a real socket, a recording
# helper, and stand-ins for the notifier and the clipboard.
if [[ -x target/debug/tormarchyd ]]; then
  python3 tests/daemon/scenarios.py target/debug/tormarchyd >/tmp/tormarchy-daemon.log 2>&1 \
    && pass "$(grep -c '  ok' /tmp/tormarchy-daemon.log) daemon scenarios pass" \
    || { grep -E 'FAIL|Error|Traceback' -A2 /tmp/tormarchy-daemon.log | head -20; fail "daemon scenarios failed"; }
else
  fail "no daemon to test"
fi

group "Commands"

"$bin" help >/dev/null 2>&1 && pass "help runs" || fail "help does not run"
PKEXEC_UID=0 "$bin" setup >/dev/null 2>&1 && fail "setup ran through polkit" \
  || pass "setup refuses to run through polkit"
PKEXEC_UID=0 "$bin" browser-exec -- true >/dev/null 2>&1 && fail "browser-exec ran through polkit" \
  || pass "browser-exec refuses to run through polkit"

group "Result"
if (( failures == 0 )); then
  echo "  everything passed"
else
  echo "  $failures check(s) failed"
  exit 1
fi
