#!/usr/bin/env bash
#
# Loads the widget under a real Qt engine against Omarchy's own shell, in every
# state it can be in, then drives the Service through its scenarios.
#
#   OMARCHY_PATH=~/src/omarchy .github/scripts/shell.sh
#   OMARCHY_PATH="~/omarchy-4.0.4 ~/src/omarchy" .github/scripts/shell.sh
#
# check.sh covers the code; this covers the one failure nothing else can see:
# the shell refusing to load the widget at all, which leaves the bar without it
# and says so only in the shell's own log. Needs PySide6 and an Omarchy
# checkout. PYTHON picks the interpreter, for a venv.
#
# OMARCHY_PATH may name several checkouts, and each is run in turn. Run the
# latest release as well as the development branch: the widget once named a type
# only the development branch had, and on every released Omarchy it simply
# never appeared in the bar.

set -uo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/../.." || exit 1

python=${PYTHON:-python3}
fixtures=tests/qml/fixtures
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT

failures=0
load() {          # load <what> <load.py arguments...>
  local what=$1; shift
  echo "== $what"
  "$python" tests/qml/load.py --omarchy "$omarchy" --panel "$out/panel.png" "$@" \
    || failures=$(( failures + 1 ))
}

read -ra omarchies <<<"${OMARCHY_PATH:-/usr/share/omarchy}"
for omarchy in "${omarchies[@]}"; do
  echo "#### $omarchy"
  load "not installed"
  for state in on off boot blocked socks; do
    load "$state" --status "$fixtures/$state.json" --bar "$out/bar.png"
  done
  load "leak check, passing" --status "$fixtures/on.json" --doctor "$fixtures/doctor-pass.json" \
    --expand leaks --call tor.checkLeaks
  load "leak check, failing" --status "$fixtures/on.json" --doctor "$fixtures/doctor-fail.json" \
    --expand leaks --call tor.checkLeaks
  load "bridges, one transport missing" --status "$fixtures/on.json" --bridges "$fixtures/bridges-missing.json" \
    --expand bridges --call tor.loadBridgeDetail
  load "bridges, a paste to preview" --status "$fixtures/on.json" --clipboard "$fixtures/bridges.txt" \
    --bridge-check "$fixtures/bridge-check-missing.json" --expand bridges --call tor.pasteBridges

  echo "== scenarios"
  "$python" tests/qml/scenarios.py --omarchy "$omarchy" || failures=$(( failures + 1 ))
done

echo
if (( failures == 0 )); then
  echo "  loads in the shell, in every state"
else
  echo "  $failures run(s) failed"
  exit 1
fi
