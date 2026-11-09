# Contributing

Pull requests welcome. Run the checks before you open one:

```bash
.github/scripts/check.sh        # or tests/run
```

CI runs that exact script, so green locally means green on the PR. It needs Rust
and Python. Run it under `sudo` to also validate the firewall ruleset against
the kernel and run the kill switch in network namespaces — skipped otherwise,
and it's the part that matters most.

## Layout

| | |
|---|---|
| `crates/tormarchy-core` | Everything the two programs share: the control-port client, the ruleset, IPv6, bridges, exits, the leak check, the menu file, and in `view.rs` every word the panel shows. |
| `crates/tormarchy` | The command line and the root helper: dispatch, the pkexec allow-list, `setup` and `uninstall` with their files compiled in, browser-only. |
| `crates/tormarchyd` | The panel's daemon: JSON lines on stdin and stdout, one control-port session with events, the state machine behind the switch, notifications, latency, bridges. |
| `setup` | The bootstrap: fetch the release, verify it, hand over. The only shell script that ships. |
| `Panel.qml` | The bar icon and the panel. Draws `tormarchyd`'s view, sends clicks back. |
| `Service.qml` | Runs `tormarchyd`. The manifest makes it the plugin's service, so the shell runs one for every monitor's widget. |
| `TorIcon` / `TorWordmark` / `SpeedGauge` | Drawn in QML, no image files. |
| `etc/` | What `setup` installs, compiled into the helper: torrc snippet, polkit policy and rule, tmpfiles, completion. |
| `tests/` | The daemon scenarios, the Qt load and wiring checks, the menu check, the kernel kill switch, pinned decisions. Unit tests live beside the code in `crates/`. |

## Working on it

```bash
ln -s "$PWD" ~/.config/omarchy/plugins/tripleu.tor
omarchy plugin enable tripleu.tor
make install-dev      # build, then install that build -- unverified, for you only
```

`make install-dev` is `sudo ./setup --local` on a static release build: the same
install a user gets, minus the download and the signature check. Run it again
after changing Rust; `omarchy-restart-shell` after changing QML. Saving a file
does **not** hot-reload: with a symlinked checkout your edits land at the link's
target, so Omarchy's watcher never fires.

To see what the shell thought of your QML:

```bash
grep -i error /run/user/$UID/quickshell/by-pid/$(pgrep -f 'quickshell.*omarchy/shell' | head -1)/log.log
```

## The daemon

`tests/daemon/scenarios.py` runs the real `tormarchyd` against a control port on
a real socket, a helper that records what it was asked, and stand-ins for the
notifier and the clipboard: that the switch reacts before tor does, that a
connect can be cancelled while it waits, that a tor restart is not mistaken for
the kill switch, that each notification replaces the last, that bridge lines
reach the helper over stdin and never a command line.

The daemon takes three variables for this and nothing else does:
`TORMARCHY_ROOT`, `TORMARCHY_HELPER` and `TORMARCHY_CONTROL_PORT`. It runs as
you, so they reach nothing you couldn't already; the root helper reads none of
them, and `test_decisions.sh` holds it to that.

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
  --view tests/qml/fixtures/view-on.json --bar bar.png --panel panel.png
```

`--view` is what `tormarchyd` would send, and `--bar`/`--panel` save what the bar
icon and the open panel look like; `--expand leaks|bridges` opens a tool. The
fixtures in `tests/qml/fixtures` are the daemon's own output for each state, and
`cargo test` fails if they ever differ from it, so the panel is never tested
against a view the daemon would not send. After changing the view, regenerate
them with `TORMARCHY_UPDATE_FIXTURES=1 cargo test`.

`tests/qml/scenarios.py` checks the wiring that is left in QML: the daemon
started once by its fixed path, told the widget's settings, handed each click; a
missing helper read as not installed and retried with a backoff.
`.github/scripts/shell.sh` runs all of it, in every state, and CI runs that
weekly against the current Omarchy and Qt:

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
cargo run -q -p tormarchy -- connect --dry-run strict | sudo nft -c -f -
```

`check.sh` has regression tests for leaks that actually shipped — DNS escaping
to the LAN router, `.onion` UDP escaping through the `10.0.0.0/8` exemption,
forwarded traffic bypassing the output hook entirely — and so do the ruleset's
own unit tests. If you reorder rules and one of those fails, it is not the test
being fussy.

The ordering guarantees around the rules -- panic landing mid-apply, rules down
before tor stops, IPv6 saved once per connection -- are tested in
`crates/tormarchy-core/src/ops.rs` against a fake machine that misbehaves on cue.

`tests/kernel/killswitch.sh` puts the real ruleset on in a network namespace and
proves, in a real kernel, that nothing reaches the wire and that a connection
open before the switch fails at once. `check.sh` runs it when it has root and
namespaces; elsewhere it skips.

`tests/menu.js` runs the helper's `menu add` and checks Tor's rows with the
menu's own parser, from the Omarchy checkout you give it:
`cargo build && node tests/menu.js ~/src/omarchy`.
`shell.sh` runs it against every checkout it is given.

## Screenshots

The README's pictures and `preview.png`, the marketplace card, are rendered from
the QML rather than taken by hand, so they show what the code draws, in
Omarchy's default theme and font. After changing the panel:

```bash
OMARCHY_PATH=~/src/omarchy make screenshots
```

## If you touch the panel's tools

The leak check makes real requests, so it runs when asked and never from a
`Timer`. Bridge lines go to the helper over stdin only — never argv, which any
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

After that, committing anything that changes what ships bumps the patch version
in `manifest.json`, `Cargo.toml` and `Cargo.lock` together, and pushing to
master builds, signs and publishes the release. Docs-only commits don't bump, so
they don't produce one.

For a minor or major version, edit `version` in `manifest.json` yourself — the
hook carries a hand-set version to the binaries and leaves it alone otherwise.

The release job builds static `x86_64-unknown-linux-musl` binaries, signs the
archive keyless with the workflow's own GitHub identity, verifies that signature
exactly as `setup` will, and only then tags the commit it built and publishes.
`setup` checks the same three things: this repository, this workflow on
`master`, and the commit the tag names on GitHub. Change the workflow's name or
branch and every install fails to verify; `check.sh` holds the two together.

The bump happens at commit time rather than in a workflow because the release job
runs as the Actions bot, which has write access but isn't an administrator, and
branch protection requires the Checks status before anything lands on master. A
bot-authored bump gets rejected every time.

## Style

Match what's there. Comments explain *why* something is the way it is —
particularly where the obvious approach is wrong — not what the line does.
