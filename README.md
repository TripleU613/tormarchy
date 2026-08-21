# Tormarchy

Route **all** system traffic through Tor from the Omarchy bar. One click to
connect, a real kill switch, new circuits on demand, and exit-country choice.

> Status: early. The bar widget is in place; the privileged helpers it drives
> are landing next. See [PLAN.md](PLAN.md).

Requires Omarchy 4.x (the Quickshell `omarchy-shell`).

## Install

```bash
omarchy plugin add https://github.com/<you>/tormarchy.git --enable
sudo ~/.config/omarchy/plugins/usher.tor/setup
```

`setup` installs `tor`, drops the privileged helpers into `/usr/local/bin`, and
registers a polkit action so connecting is one bar click and one auth prompt.

## Modes

| Mode | Behaviour |
|---|---|
| **LAN** *(default)* | All internet traffic through Tor. Non-Tor traffic dropped. Local network still reachable — printers, SSH, Syncthing. |
| **Strict** | As above, local network dropped too. Nothing leaves except through Tor. |
| **SOCKS** | No system-wide change. `tor` runs with a SOCKS proxy on 9050 and apps opt in. |

## Using it

Left click opens the panel, right click toggles the connection, middle click
requests a new circuit.

Inside the panel: `j`/`k` and arrows move, `enter`/`space` activates, `t`
toggles, `n` new circuit, `s` speed test, `r` refresh, `esc` closes.

The bar shows the exit country code when connected, and the bootstrap
percentage while it comes up. The onion only lights up once traffic is
actually going through Tor — never before.

## What this breaks

Transparent Tor has costs, and they are not subtle:

- **UDP is dropped.** Video calls, WireGuard/Tailscale, most multiplayer games.
- **IPv6 is dropped entirely.** Some networks degrade noticeably.
- **No other VPN alongside it.**
- Docker/podman bridge traffic needs explicit handling.

Use SOCKS mode if you want Tor for one browser and a working machine otherwise.

## The kill switch

If `tor` dies while connected, the firewall rules **stay** and your network
goes down. That is deliberate: a Tor kill switch that fails open is not a kill
switch. Recover with the panel's disconnect, or `tormarchy-panic` from a shell.

Nothing is written to `/etc/nftables.conf`, so **a reboot always restores
normal networking.** That is the guaranteed way out of a wedged ruleset.

## Development

```bash
ln -s "$PWD" ~/.config/omarchy/plugins/usher.tor
omarchy-shell shell rescanPlugins
omarchy plugin enable usher.tor
```

Saved files under `~/.config/omarchy/plugins/` hot-reload. Before committing:

```bash
omarchy plugin validate .
qmllint -I "$OMARCHY_PATH/shell" *.qml   # needs qt6-declarative
```

## Credits

Structure follows Omarchy's own `omarchy.dropbox` and `omarchy.tailscale`
widgets. The speed test reuses the first-party `omarchy.speedtest` panel.

## License

MIT
