# Contributing

Pull requests welcome. Run the checks before you open one:

```bash
.github/scripts/check.sh
```

CI runs that exact script, so green locally means green on the PR. Run it under
`sudo` to also validate the firewall ruleset against the kernel — that part is
skipped otherwise, and it's the part that matters most.

## Layout

| | |
|---|---|
| `tormarchy` | Everything that touches the system. One file, all Bash. |
| `Panel.qml` | The bar icon and the panel. |
| `Service.qml` | Talks to `tormarchy`, holds the state the panel renders. |
| `Model.js` | Parsing and formatting. No Qt in here, so it's testable with `node`. |
| `TorIcon` / `TorWordmark` / `SpeedGauge` | Drawn in QML, no image files. |
| `etc/` | What `setup` installs: torrc snippet, polkit policy and rule, tmpfiles. |
| `tests/` | Pinned decisions, helper behaviour, Model.js, the Qt load check and the Service scenarios. |

## Working on it

```bash
ln -s "$PWD" ~/.config/omarchy/plugins/tripleu.tor
omarchy plugin enable tripleu.tor
```

Saving a file does **not** hot-reload. Omarchy watches
`~/.config/omarchy/plugins/`, and with a symlinked checkout your edits land at
the link's target, so the watcher never fires. Run `omarchy-restart-shell` after
every change.

To see what the shell thought of your QML:

```bash
grep -i error /run/user/$UID/quickshell/by-pid/$(pgrep -f 'quickshell.*omarchy/shell' | head -1)/log.log
```

## Loading it without a shell

The shell loads the widget with `Qt.createComponent` and, if that fails, logs a
line and leaves it out of the bar. The plugin still installs and enables, so the
only symptom is that nothing appears. `tests/qml/load.py` reproduces that load
off the machine, against a real Qt engine and Omarchy's own `qs.Commons` and
`qs.Ui`, and fails on any error or warning the plugin's QML raises:

```bash
pip install PySide6
python3 tests/qml/load.py --omarchy ~/src/omarchy            # or /usr/share/omarchy
python3 tests/qml/load.py --omarchy ~/src/omarchy \
  --status tests/qml/fixtures/on.json --bar bar.png --panel panel.png
```

The fixtures stand in for `tormarchy status --json` in each state, and
`--bar`/`--panel` save what the bar icon and the open panel look like. Others
answer the panel's tools — `--doctor`, `--bridges`, `--bridge-check`,
`--clipboard` — and `--expand leaks|bridges` with `--call tor.checkLeaks` (or
any Service or panel function) opens and drives one. `--font` loads a Nerd Font
so the glyphs render, `--log` prints every command the plugin ran and every
byte it wrote to a stdin.

`tests/qml/scenarios.py` drives the Service through sequences of states: that a
tor restart is not mistaken for the kill switch, that each notification fires
once and replaces the last, that bridge lines reach `tormarchy` over stdin and
never appear on a command line. `.github/scripts/shell.sh` runs all of it, in
every state, and CI runs that weekly against the current Omarchy and Qt:

```bash
OMARCHY_PATH=~/src/omarchy PYTHON=~/venvs/qt/bin/python make shell
```

Read palette colours through the alias, `import qs.Commons as Commons` then
`Commons.Color.foreground`. Qt 6.12 has a `Color` of its own, and a bare
`Color.foreground` now quietly resolves to it and comes back undefined.

## If you touch the firewall

The ruleset is the part that can take someone's network down, so it gets more
care than the rest:

```bash
sudo ./tormarchy connect --dry-run strict | sudo nft -c -f -
```

`check.sh` has regression tests for leaks that actually shipped — DNS escaping
to the LAN router, `.onion` UDP escaping through the `10.0.0.0/8` exemption,
forwarded traffic bypassing the output hook entirely. If you reorder rules and
one of those fails, it is not the test being fussy.

`tests/test_helpers.sh` runs the script's own functions with the machine stubbed
out — IPv6 save and restore, the boot marker, circuit selection and relay
details, exit exclusions, bridge parsing, the doctor and status JSON. The IPv6
and bridge-summary cases need root and skip themselves without it.

`tests/kernel/killswitch.sh` puts the real ruleset on in a network namespace and
proves, in a real kernel, that nothing reaches the wire and that a connection
open before the switch fails at once. `check.sh` runs it when it has root and
namespaces; elsewhere it skips.

`tests/menu.js` checks Tor's rows in the Omarchy menu with the menu's own parser,
from the Omarchy checkout you give it: `node tests/menu.js ~/src/omarchy`.
`shell.sh` runs it against every checkout it is given.

## If you touch the panel's tools

The leak check makes real requests, so it runs when asked and never from a
`Timer`. Bridge lines go to `tormarchy` over stdin only — never argv, which any
process can read in `/proc`, and never a path the root helper is handed.
Notifications go through `omarchy-notification-send`, never `notify-send`.
`tests/test_decisions.sh` fails if any of those change.

Rules only ever go on after Tor reports a full bootstrap, and nothing is
persisted to `/etc/nftables.conf`, so a reboot always restores normal
networking. Please keep both of those true.

## Releases

Automatic. Enable the hook once per clone:

```bash
make hooks
```

After that, committing anything that changes what ships bumps the patch version,
and pushing to master tags it and publishes a release. Docs-only commits don't
bump, so they don't produce a release.

For a minor or major version, edit `version` in `manifest.json` yourself — the
hook only touches the patch field, and it leaves a hand-set version alone if it
has no tag yet.

The bump happens at commit time rather than in a workflow because the release job
runs as the Actions bot, which has write access but isn't an administrator, and
branch protection requires the Checks status before anything lands on master. A
bot-authored bump gets rejected every time.

## Style

Match what's there. Comments explain *why* something is the way it is —
particularly where the obvious approach is wrong — not what the line does.
