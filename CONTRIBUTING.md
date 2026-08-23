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

## Working on it

```bash
ln -s "$PWD" ~/.config/omarchy/plugins/usher.tor
omarchy plugin enable usher.tor
```

Saving a file does **not** hot-reload. Omarchy watches
`~/.config/omarchy/plugins/`, and with a symlinked checkout your edits land at
the link's target, so the watcher never fires. Run `omarchy-restart-shell` after
every change.

To see what the shell thought of your QML:

```bash
grep -i error /run/user/$UID/quickshell/by-pid/$(pgrep -f 'quickshell.*omarchy/shell' | head -1)/log.log
```

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

Rules only ever go on after Tor reports a full bootstrap, and nothing is
persisted to `/etc/nftables.conf`, so a reboot always restores normal
networking. Please keep both of those true.

## Releases

Tag it. The release workflow refuses to publish if the tag and the `version` in
`manifest.json` disagree.

```bash
# bump "version" in manifest.json first
git tag v1.0.0 && git push origin v1.0.0
```

## Style

Match what's there. Comments explain *why* something is the way it is —
particularly where the obvious approach is wrong — not what the line does.
