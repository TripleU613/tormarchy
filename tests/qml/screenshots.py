#!/usr/bin/env python3
"""Every picture of the plugin -- the README's and the marketplace preview --
rendered from the real QML, the way the Qt check loads it.

    python3 tests/qml/screenshots.py --omarchy ~/src/omarchy

Screenshots taken by hand go stale with every change and are taken on whatever
theme and font the person happened to run. These come from the fixtures, at
twice the size so edges stay sharp, in Omarchy's default theme and font:

    assets/screenshots/panel.png        connected
    assets/screenshots/leak-check.png   the leak check, run
    assets/screenshots/bridges.png      a bridge paste, previewed
    assets/screenshots/bar.png          the bar icon in each state
    preview.png                         the marketplace card, 1600x900

Needs PySide6, an Omarchy checkout, and JetBrains Mono installed. Pass the Nerd
Font symbols with --font unless a JetBrainsMono Nerd Font is installed.
"""
import argparse
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FIXTURES = os.path.join(HERE, "fixtures")

ap = argparse.ArgumentParser()
ap.add_argument("--omarchy", default=os.environ.get("OMARCHY_PATH", "/usr/share/omarchy"))
ap.add_argument("--theme", default="tokyo-night", help="a theme under <omarchy>/themes")
ap.add_argument("--font", action="append", default=[], help="extra font file, e.g. the Nerd Font symbols")
ap.add_argument("--out", default=os.path.join(REPO, "assets", "screenshots"))
ap.add_argument("--preview", default=os.path.join(REPO, "preview.png"))
args = ap.parse_args()

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
from PySide6.QtCore import QPointF, QRectF, Qt  # noqa: E402
from PySide6.QtGui import (QColor, QFont, QFontDatabase, QGuiApplication, QImage,  # noqa: E402
                           QLinearGradient, QPainter, QPainterPath)
from PySide6.QtSvg import QSvgRenderer  # noqa: E402

theme_dir = os.path.join(os.path.abspath(os.path.expanduser(args.omarchy)), "themes", args.theme)
palette = {}
for line in open(os.path.join(theme_dir, "colors.toml")):
    key, _, value = line.partition("=")
    if value.strip().startswith('"#'):
        palette[key.strip()] = value.strip().strip('"')

families = subprocess.run(["fc-list", ":", "family"], capture_output=True, text=True).stdout
family = "JetBrainsMono Nerd Font" if "JetBrainsMono Nerd Font" in families else "JetBrains Mono"
if "JetBrains Mono" not in families:
    sys.exit("JetBrains Mono is not installed -- the pictures would not look like Omarchy")

work = tempfile.mkdtemp(prefix="tormarchy-shots-")
os.makedirs(args.out, exist_ok=True)


def render(name, *extra, env=None):
    """One load.py run at twice the size; returns the cropped card and bar."""
    card, bar = os.path.join(work, name + "-card.png"), os.path.join(work, name + "-bar.png")
    cmd = [sys.executable, os.path.join(HERE, "load.py"), "--omarchy", args.omarchy,
           "--theme", theme_dir, "--font-family", family, "--scale", "2",
           "--panel", os.path.join(work, name + ".png"), "--card", card, "--bar", bar]
    for font in args.font:
        cmd += ["--font", font]
    # A latency on the fast side of normal, as a healthy connection reads.
    result = subprocess.run(cmd + list(extra), capture_output=True, text=True,
                            env={**os.environ, "TORMARCHY_STUB_PING": "380", **(env or {})})
    if "result: ok" not in result.stdout:
        sys.exit(f"{name} did not render cleanly:\n{result.stdout}{result.stderr}")
    return card, bar


on = os.path.join(FIXTURES, "on.json")
shots = {
    "panel": render("panel", "--status", on),
    "leak-check": render("leak-check", "--status", on, "--doctor", os.path.join(FIXTURES, "doctor-pass.json"),
                         "--expand", "leaks", "--call", "tor.checkLeaks"),
    "bridges": render("bridges", "--status", on, "--clipboard", os.path.join(FIXTURES, "bridges.txt"),
                      "--bridge-check", os.path.join(FIXTURES, "bridge-check-ok.json"),
                      "--expand", "bridges", "--call", "tor.pasteBridges"),
}
# In the order the README's table lists them. Connecting is a switch flipped
# on and still settling, so it is that: the off state, with the switch flipped.
states = [
    render("bar-on", "--status", on)[1],
    render("bar-connecting", "--status", os.path.join(FIXTURES, "off.json"), "--call", "tor.toggleConnected")[1],
    render("bar-socks", "--status", os.path.join(FIXTURES, "socks.json"))[1],
    render("bar-off", "--status", os.path.join(FIXTURES, "off.json"))[1],
    render("bar-blocked", "--status", os.path.join(FIXTURES, "blocked.json"))[1],
]

app = QGuiApplication(sys.argv[:1])
for font in args.font:
    QFontDatabase.addApplicationFont(os.path.abspath(font))

for name, (card, _) in shots.items():
    QImage(card).save(os.path.join(args.out, name + ".png"))
    print("saved", os.path.join(args.out, name + ".png"))

# The bar icon in every state, left to right: on, connecting, browser-only, off,
# kill switch holding -- the README's table, row by row.
icons = [QImage(path) for path in states]
gap = 24
strip = QImage(sum(i.width() for i in icons) + gap * (len(icons) + 1), icons[0].height() + 16, QImage.Format_RGB32)
strip.fill(QColor(palette["background"]))
p = QPainter(strip)
x = gap
for icon in icons:
    p.drawImage(x, 8, icon)
    x += icon.width() + gap
p.end()
strip.save(os.path.join(args.out, "bar.png"))
print("saved", os.path.join(args.out, "bar.png"))

# The marketplace card: the mark and the name on the left, the panel open
# beneath its icon in a bar on the right, the way it sits on a desktop.
W, H = 1600, 900
preview = QImage(W, H, QImage.Format_RGB32)
p = QPainter(preview)
p.setRenderHint(QPainter.Antialiasing)
p.setRenderHint(QPainter.SmoothPixmapTransform)

backdrop = QLinearGradient(0, 0, W, H)
backdrop.setColorAt(0, QColor(palette.get("darker_background", palette["background"])))
backdrop.setColorAt(1, QColor(palette.get("lighter_background", palette["background"])))
p.fillRect(0, 0, W, H, backdrop)

panel = QImage(shots["leak-check"][0])
scale = 0.75                     # 2x renders shown at 1.5x
bar_h = round(26 * 2 * scale)
card_w, card_h = round(panel.width() * scale), round(panel.height() * scale)
card_x, card_y = W - 64 - card_w, bar_h + 18

p.fillRect(0, 0, W, bar_h, QColor(palette["background"]))

glyphs = QFont(family)
glyphs.setPixelSize(round(13 * 2 * scale))
text = QFont(family)
text.setPixelSize(round(12 * 2 * scale))

# Workspaces on the left, the clock in the middle, as Omarchy's bar has them.
p.setFont(text)
for n, ws in enumerate("12345"):
    p.setPen(QColor(palette["foreground"] if n == 0 else palette.get("muted", palette["foreground"])))
    p.drawText(QRectF(28 + n * 34, 0, 34, bar_h), Qt.AlignCenter, ws)
p.setPen(QColor(palette["foreground"]))
p.drawText(QRectF(0, 0, W, bar_h), Qt.AlignCenter, "Mon 14:02")

# The tray: Tor first, under the panel it opened, then network, sound, battery.
# Just the slot: the crop around it is taller than the bar.
tor_icon = QImage(states[0])
tor_icon = tor_icon.copy(0, (tor_icon.height() - 52) // 2, tor_icon.width(), 52)
tor_cx = card_x + card_w / 2
icon_w = tor_icon.width() * scale
p.drawImage(QRectF(tor_cx - icon_w / 2, (bar_h - tor_icon.height() * scale) / 2, icon_w, tor_icon.height() * scale),
            tor_icon)
p.fillRect(QRectF(tor_cx - 11, bar_h - 5, 22, 3), QColor(palette["accent"]))
p.setFont(glyphs)
p.setPen(QColor(palette["foreground"]))
slot = 27 * 2 * scale
for n, glyph in enumerate(("\U000F0928", "\U000F057E", "\U000F0079")):
    p.drawText(QRectF(tor_cx + slot * (n + 0.5), 0, slot, bar_h), Qt.AlignCenter, glyph)

p.fillRect(0, bar_h - 1, W, 1, QColor(palette.get("selection", palette["background"])))

# A soft shadow under the panel, then the panel.
for spread, alpha in ((22, 18), (14, 26), (7, 38)):
    shadow = QPainterPath()
    shadow.addRoundedRect(QRectF(card_x - spread, card_y - spread + 8, card_w + 2 * spread, card_h + 2 * spread),
                          18, 18)
    p.fillPath(shadow, QColor(0, 0, 0, alpha))
p.drawImage(QRectF(card_x, card_y, card_w, card_h), panel)

# The name. Drawn at exactly 2x, where every one of the wordmark's blocks lands
# on whole pixels -- at a fractional scale each one antialiases into a faint
# seam against its neighbour -- then scaled down whole.
logo = QSvgRenderer(os.path.join(REPO, "assets", "logo.svg"))
size = logo.defaultSize()
logo_2x = QImage(size.width() * 2, size.height() * 2, QImage.Format_ARGB32_Premultiplied)
logo_2x.fill(Qt.transparent)
lp = QPainter(logo_2x)
logo.render(lp, QRectF(0, 0, logo_2x.width(), logo_2x.height()))
lp.end()
logo_w, logo_h = size.width() * 1.5, size.height() * 1.5
logo_x = (card_x - logo_w) / 2
logo_y = H / 2 - logo_h - 10
p.drawImage(QRectF(logo_x, logo_y, logo_w, logo_h), logo_2x)

tagline = QFont(family)
tagline.setPixelSize(34)
p.setFont(tagline)
p.setPen(QColor(palette.get("bright_foreground", palette["foreground"])))
p.drawText(QRectF(0, logo_y + logo_h + 40, card_x, 48), Qt.AlignCenter, "Tor for the Omarchy bar")
features = QFont(family)
features.setPixelSize(22)
p.setFont(features)
p.setPen(QColor(palette.get("dark_foreground", palette["foreground"])))
p.drawText(QRectF(0, logo_y + logo_h + 98, card_x, 34), Qt.AlignCenter,
           "kill switch · leak check · bridges · exit picker")
p.end()
preview.save(args.preview)
print("saved", args.preview)
