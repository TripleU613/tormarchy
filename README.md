# Tormarchy

Route **all** system traffic through Tor from the Omarchy bar. One click to
connect, a real kill switch, new circuits on demand, and exit-country choice.

Requires Omarchy 4.x (the Quickshell `omarchy-shell`).

## Install

```bash
omarchy plugin add https://github.com/TripleU613/tormarchy.git --enable
sudo ~/.config/omarchy/plugins/usher.tor/tormarchy setup
```

`setup` installs `tor`, puts the `tormarchy` command in `/usr/local/bin`, and
registers a polkit action so connecting is one bar click and no password.

It runs from a terminal only — `setup` and `uninstall` refuse to run through
polkit, because the single polkit action pins the program path and not its
arguments, so a passwordless grant for connecting would otherwise cover
uninstalling too.

## Modes

| Mode | Behaviour |
|---|---|
| **Maximum** | Every packet through Tor. Local network dropped too. Nothing leaves except through Tor. |
| **Standard** *(default)* | All internet traffic through Tor, non-Tor traffic dropped, local network still reachable — printers, SSH, Syncthing. |
| **Browser only** | No system-wide change at all. `tormarchy-browser` launches a browser that can reach *nothing* but Tor; the rest of the machine is untouched. |

Maximum and Standard also turn off the kernel's IPv6 stack and drop everything
this machine would otherwise **forward** — a bridged VM, a container network, a
shared hotspot or a tethered phone never touches the output hook, so without a
forward chain that traffic leaves the NIC untouched while the onion is lit.

### Browser only is enforced, not requested

Setting a browser's proxy preference is a request. WebRTC, QUIC, captive-portal
probes and extensions all ignore it, and the browser keeps working, so nothing
looks wrong. `tormarchy-browser` instead launches it in a systemd scope with:

```
IPAddressDeny=any
IPAddressAllow=localhost
```

systemd applies that as a BPF filter on the process's cgroup, so the browser
cannot open a socket to any address except loopback — whatever it tries. Point
its proxy at Tor on loopback and Tor is its only route. A leak attempt doesn't
slip through, it fails.

Works with any installed browser: Firefox-based ones get a dedicated profile
with remote DNS, WebRTC and DoH off; Chromium-based ones get `--proxy-server`
plus `--host-resolver-rules` so DNS can't leak; anything else falls back to
proxy environment variables, which is safe *because* of the address filter.

## Commands

| | |
|---|---|
| `tormarchy-connect [mode]` | Connect. `--dry-run` prints the ruleset without applying it. |
| `tormarchy-disconnect` | Remove the rules and stop tor. |
| `tormarchy-browser [name]` | Launch a browser that can reach only Tor. `--list` shows what's installed. |
| `tormarchy-ip` | Show the current circuit path and exit, read from Tor's control port. Makes no network request. |
| `tormarchy-newnym` | New circuit. |
| `tormarchy-exit <cc>` | Pin an exit country. `--list` shows which have exits. |
| `tormarchy-bridge` | Use bridges on a network that blocks Tor. |
| `tormarchy-doctor` | Leak checks, including an IPv6 test where *success is a failure*. |
| `tormarchy-panic` | Remove every rule, unconditionally. The escape hatch. |

## Using it

Left click opens the panel, right click toggles the connection, middle click
requests a new circuit.

Inside the panel: `j`/`k` and arrows move, `enter`/`space` activates, `t`
toggles, `n` new circuit, `e` exit country, `s` measure, `r` refresh, `esc`
closes.

The bar shows only the onion. It lights up once traffic is actually going
through Tor — never before.

## What this breaks

Transparent Tor has costs, and they are not subtle:

- **UDP is dropped.** Video calls, WireGuard/Tailscale, most multiplayer games.
- **IPv6 is dropped entirely**, and the kernel stack is switched off.
- **Nothing is forwarded.** A VM or container that routes through this host
  loses its network in Maximum and Standard mode. That is the point.
- **No other VPN alongside it.**
- **NTP is blocked**, so a long session can drift its own clock far enough that
  Tor stops building circuits. `tormarchy-doctor` checks for this.

Use Browser only if you want Tor for browsing and a working machine otherwise.

## What Tor does not fix

Worth being blunt, because a lit onion invites the wrong assumption:

Tor changes **where your traffic comes from**, not **who a site already knows
you are**. A logged-in account stays logged in. A browser profile keeps its
cookies, its history and its fingerprint. Connecting mid-session doesn't undo
any of that — `tormarchy-connect` lists programs holding non-Tor connections
before it changes anything, for exactly this reason.

If you need the two separated, use Browser only, which launches a browser with
its own profile.

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

Note that saving a file **does not** hot-reload here. Omarchy watches
`~/.config/omarchy/plugins/`, and a symlinked checkout means edits happen at the
link's target, so the watcher never fires and `rescanPlugins` has nothing to
pick up. Use `omarchy-restart-shell` after every change.

Before committing:

```bash
bash -n tormarchy                        # 1200 lines, one typo away from silent
xmllint --noout etc/polkit/com.tormarchy.policy
omarchy plugin validate .
sudo ./tormarchy connect --dry-run strict | sudo nft -c -f -
```

That last one matters most: the ruleset is the part that can take your network
down, and `nft -c` checks it against the kernel without applying anything.

## Credits

Structure follows Omarchy's own `omarchy.dropbox` and `omarchy.tailscale`
widgets. The speed test reuses the first-party `omarchy.speedtest` panel.

## License

MIT
