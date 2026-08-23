#!/bin/bash
#
# Every check CI runs, in one script you can run yourself:
#
#   .github/scripts/check.sh
#
# CI calls this same file, so a green run here means a green run there. Nothing
# in it needs root, Omarchy, or Quickshell -- it works on a bare runner and on a
# contributor's laptop.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 1

failures=0
pass() { printf '  \033[32mok\033[0m    %s\n' "$1"; }
fail() { printf '  \033[31mFAIL\033[0m  %s\n' "$1"; failures=$(( failures + 1 )); }
note() { printf '  \033[90m--\033[0m    %s\n' "$1"; }
group() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# ---------------------------------------------------------------------- shell --

group "Shell"

if bash -n tormarchy 2>/dev/null; then
  pass "tormarchy parses"
else
  bash -n tormarchy
  fail "tormarchy has a syntax error"
fi

if command -v shellcheck >/dev/null 2>&1; then
  # Errors block; warnings and below are printed but do not fail the build.
  # A 1200-line script accumulates stylistic warnings that are not worth
  # gating a contributor's pull request on, but a genuine error is.
  if shellcheck --severity=error tormarchy; then
    pass "shellcheck reports no errors"
  else
    fail "shellcheck reports errors"
  fi
  shellcheck --severity=warning tormarchy >/dev/null 2>&1 \
    || note "shellcheck has warnings (advisory, not blocking): shellcheck tormarchy"
else
  note "shellcheck not installed, skipping lint"
fi

[[ -x tormarchy ]] && pass "tormarchy is executable" || fail "tormarchy is not executable (chmod +x)"

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
for path in sorted(glob.glob("*.qml")) + ["Model.js"]:
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

print(f"  \033[32mok\033[0m    {len(glob.glob('*.qml')) + 1} QML/JS files are balanced")
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
  ./tormarchy connect --dry-run "$mode" >"/tmp/ruleset-$mode.nft" 2>/dev/null || true
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

# --------------------------------------------------------------------- shape --

group "Commands"

missing=""
for sub in help status ip newnym exit bridge browser speed doctor panic connect disconnect mode setup uninstall pingd; do
  # The pattern allows "status)" and "help | -h | --help)" alike.
  grep -qE "^${sub}[)| ]" tormarchy || missing+=" $sub"
done
if [[ -z $missing ]]; then
  pass "every documented subcommand is dispatched"
else
  fail "dispatcher is missing:$missing"
fi

./tormarchy help >/dev/null 2>&1 && pass "help runs" || fail "help does not run"

group "Result"
if (( failures == 0 )); then
  echo "  everything passed"
else
  echo "  $failures check(s) failed"
  exit 1
fi
