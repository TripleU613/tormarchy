# Tormarchy — Tor toggle for the Omarchy bar

One click in the top bar routes **all** system traffic through Tor. New circuit,
exit country, speed test, and a real kill switch.

Target: Omarchy 4.x (Quickshell `omarchy-shell`). Ships as a third-party plugin
installed with `omarchy plugin add`, because `omarchy-plugin-validate` rejects
any id in the reserved `omarchy.*` namespace.

## Stack

| Layer | Language | Precedent in Omarchy |
|---|---|---|
| Bar widget + panel | QML + JS | `omarchy.tailscale` (`Service.qml` / `Panel.qml` / `Model.js` / `TailscaleIcon.qml`) |
| System + network | Bash | `omarchy-dns` (privileged network mutation, `sudo`-if-tty-else-`pkexec`) |

No new runtime, no build step. `git clone` and it runs.

## Layout

```
tormarchy/
  manifest.json           # id "usher.tor", kinds ["bar-widget"], defaultSection "right"
  Panel.qml               # bar indicator + popup panel
  Service.qml             # polls status, owns optimistic toggle state
  Model.js                # parse bootstrap / circuit / country output
  TorIcon.qml             # onion drawn natively in QML (tailscale draws its mark, no SVG)
  README.md               # includes a "what this breaks" section
  bin/                    # installed to /usr/local/bin by setup
    tormarchy-setup       # one-time root install
    tormarchy-status      # --json; unprivileged
    tormarchy-connect     # [mode]; privileged
    tormarchy-disconnect  # privileged
    tormarchy-mode        # strict|lan|socks; privileged
    tormarchy-exit        # <CC>|auto; privileged
    tormarchy-newnym      # unprivileged, control-port cookie
    tormarchy-doctor      # leak checks
    tormarchy-panic       # flush every rule, unconditionally
  etc/
    torrc.d/00-tormarchy.conf
    polkit/49-tormarchy.rules
```

## The three modes

Selectable in the panel via `Ui/ButtonGroup.qml`. Persisted in the plugin's
`barWidget.defaults` / `shell.json` widget settings.

| Mode | Behaviour |
|---|---|
| **LAN** (default) | All WAN through Tor, non-Tor traffic dropped, RFC1918 still reachable (printers, SSH, Syncthing). |
| **Strict** | Same, LAN dropped too. Nothing leaves except through Tor. |
| **SOCKS** | No system-wide change. `tor` runs, SOCKS on 9050, apps opt in. Zero-risk mode. |

## Transparent proxy mechanics

1. `tor` with `TransPort 9040`, `DNSPort 9053`, `SocksPort 9050`,
   `VirtualAddrNetworkIPv4 10.192.0.0/10`, `AutomapHostsOnResolve 1`,
   `ControlPort 9051` + `CookieAuthentication 1` +
   `CookieAuthFileGroupReadable 1`.
2. A dedicated `table inet tormarchy`, never touching the user's rules:
   - exempt `meta skuid tor` (or tor loops into its own redirect) and `lo`
   - redirect all TCP -> `127.0.0.1:9040`, all port-53 -> `127.0.0.1:9053`
   - **drop everything else**: UDP, QUIC, ICMP, and all IPv6
3. This box runs `ufw` (enabled). A separate nft table is correct: a DROP in any
   table wins, so the kill switch holds without editing ufw's chains.
4. Rules go on **only after** tor reports `bootstrap-phase` 100%. No window
   where traffic is redirected to a tor that isn't up yet.

## Kill-switch failure policy

- **tor dies while connected -> rules stay.** Network goes down, nothing leaks.
  Recovery is an explicit `tormarchy-disconnect` or `tormarchy-panic`. A Tor kill
  switch that fails open is not a kill switch.
- **Reboot -> rules gone.** Nothing is written to `/etc/nftables.conf`, so a
  reboot always recovers a wedged machine. This is the escape hatch.

## Feature -> mechanism

| Panel affordance | Implementation |
|---|---|
| Connect toggle | `pkexec tormarchy-connect` -> Omarchy's in-shell polkit agent (themed dialog, no terminal) |
| New circuit | `SIGNAL NEWNYM` on control port 9051, cookie auth. Instant, no restart. |
| Exit location | `SearchableDropdown` of countries -> `ExitNodes {cc}` + `StrictNodes 1` in a torrc.d snippet, then reload |
| Speed | `bar.shell.summon("omarchy.speedtest", ...)` — reuses the first-party panel, measures the live circuit |
| Status | bootstrap %, exit IP, exit country, circuit age, from `tormarchy-status --json` |
| Allow LAN | the Mode picker (LAN vs Strict) |

Bar indicator states: off / bootstrapping *(with %)* / connected *(country code)* / error.
Left click opens the panel, right click toggles — same as `omarchy.tailscale`.

Keyboard, matching the tailscale panel: `j`/`k` move, `enter`/`space` activate,
`t` toggle, `n` new circuit, `e` exit country, `s` speed, `r` refresh, `esc` close.

## Privileged install

One-time `sudo ./setup`: `pacman -S tor`, install `/usr/local/bin/tormarchy-*`,
drop the polkit action pinned to those absolute paths, add
`%include /etc/tor/torrc.d/*.conf` to `/etc/tor/torrc` if absent, add the user
to the `tor` group for control-port cookie reads.

After that, connecting is one bar click and one polkit prompt. No terminal.

## Development loop

```bash
ln -s ~/tormarchy ~/.config/omarchy/plugins/usher.tor   # -L in the catalog walk follows it
omarchy-shell shell rescanPlugins
omarchy plugin enable usher.tor
```
Saves under `~/.config/omarchy/plugins/` hot-reload. `omarchy plugin validate .`
before every commit — it mirrors the checks the shell enforces.

## What this breaks (inherent to Tor, not to this design)

- **UDP is gone.** Video calls, WireGuard/Tailscale, most multiplayer games.
- **IPv6 is dropped wholesale.** Some networks degrade noticeably.
- **Cannot run alongside another VPN.**
- Docker/podman bridge traffic needs explicit handling.

The panel says this out loud rather than letting people find out mid-meeting.

## Build order

1. Bash layer + `tormarchy-doctor` — fully testable from a terminal, no shell restarts.
2. `manifest.json` + minimal `Panel.qml` — get an onion in the bar.
3. `Service.qml` state machine + optimistic toggle.
4. Full panel: modes, exit picker, speed, keyboard nav.
5. README with captures, then propose upstream.
