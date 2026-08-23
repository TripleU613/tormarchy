<p align="center">
  <img src="assets/logo.svg" alt="Tormarchy" width="340">
</p>

<p align="center">
  A Tor toggle for the Omarchy bar. Click the onion, all your traffic goes
  through Tor. Click it again, it doesn't.
</p>

---

Needs Omarchy 4.x. Everything else it uses is already on an Arch box —
`nftables`, `systemd`, `polkit`, `curl`, `iproute2`. `setup` installs `tor`
itself. Bridges additionally want `obfs4proxy` (AUR) or `meek`, and it'll tell
you which if you need them.

## Install

```bash
omarchy plugin add https://github.com/TripleU613/tormarchy.git --enable
sudo ~/.config/omarchy/plugins/usher.tor/tormarchy setup
```

`setup` needs a terminal and tells you everything it touches as it goes. For the
record, that's: `tor` installed, `tormarchy` copied to `/usr/local/bin`, a config
snippet in `/etc/tor/torrc.d/`, one line added to `/etc/tor/torrc`, a polkit rule
so the toggle doesn't ask for a password, your user added to the `tor` group, and
`/var/lib/tor` loosened to `0750` so it can read Tor's control cookie. It doesn't
touch your Firefox or browser profiles — browser mode makes its own.

## Removing it

```bash
sudo ~/.config/omarchy/plugins/usher.tor/tormarchy uninstall
omarchy plugin remove usher.tor
```

`uninstall` puts all of the above back, including `/var/lib/tor`, and takes the
firewall rules down first so you can't end up without a network. Add `--purge` to
also drop the `tor` package and your group membership.

## Modes

| | |
|---|---|
| **Maximum** | Everything through Tor. Local network dropped too. |
| **Standard** | Everything through Tor, but printers, SSH and Syncthing still work. |
| **Browser only** | Nothing system-wide. Launches a browser that can't reach anything except Tor. |

Maximum and Standard also switch off IPv6 and refuse to forward traffic, so a VM
or container can't route around the rules.

Browser only doesn't just set a proxy preference — a proxy setting is a request
that WebRTC and QUIC happily ignore. It runs the browser under
`IPAddressDeny=any`, so the kernel won't let it open a socket to anything but
loopback. Works with any browser you have installed.

## What it costs

- **No UDP.** Video calls, WireGuard, most games.
- **No IPv6.**
- **No forwarding**, so VMs and containers lose their network.
- **No other VPN** at the same time.
- **No NTP**, so a very long session can drift its clock until Tor gives up.

Browser only avoids all of that.

And the obvious one: Tor changes where your traffic comes from, not who a site
already knows you are. If you were logged in before you connected, you're still
logged in.

## Panel


<p align="center">
  <img alt="Connected" width="46%" src="https://github.com/user-attachments/assets/f670dd75-11cd-498e-96b9-0e8a0ca544ae" />
</p>



Left click opens it, right click toggles, middle click gets a new circuit.

Inside: `j`/`k` to move, `t` toggle, `n` new circuit, `e` exit country,
`s` measure latency, `esc` to close.

The onion only lights up once traffic is actually going through Tor.

## Commands

| | |
|---|---|
| `tormarchy connect` / `disconnect` | On and off. `--dry-run` prints the firewall rules without applying them. |
| `tormarchy status` | What's happening. |
| `tormarchy ip` | Your circuit and exit, straight from Tor. Doesn't phone anyone to ask. |
| `tormarchy newnym` | New circuit. |
| `tormarchy exit de` | Leave from Germany. `--list` shows what's available. |
| `tormarchy bridge` | For networks that block Tor. |
| `tormarchy browser` | Launch a browser locked to Tor. |
| `tormarchy doctor` | Check for leaks. |
| `tormarchy panic` | Rip out every rule. For when things go wrong. |

## If it breaks

If Tor dies while you're connected, the rules stay and your network goes with
them. That's on purpose — a kill switch that fails open isn't one. Toggle it off,
or run `tormarchy panic`.

Nothing survives a reboot, so restarting always gets your network back.

## License

MIT

Not affiliated with or endorsed by the Tor Project.
