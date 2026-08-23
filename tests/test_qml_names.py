#!/usr/bin/env python3
"""Catch QML declarations that collide with a name the base type already owns.

qmllint is the real check, but it needs a Qt toolchain and it cannot resolve
qs.Commons or qs.Ui without a full Omarchy install, so its output is mostly
import noise. This runs anywhere and covers the failure mode that actually
bites: redeclaring a property a base type already defines.

`property var state` on an Item is the canonical example. QQuickItem owns
`state` for its own state machine, so the redeclaration is a hard error and
nothing catches it until the shell tries to load the plugin -- at which point
the widget silently fails to appear and the reason is buried in a log.

`property bool enabled` is worse, because it is not an error: it shadows the
property that controls whether the object receives input, so the widget loads
and then quietly ignores clicks.

Borrowed from huacnlee/omarchy-mihoro, which hit both of these for real.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Names QQuickItem itself owns, so they are unsafe in any QML file whose root
# or nested objects derive from Item -- which is all of them.
#
# Deliberately NOT including Rectangle's `color` or `radius`, or Text's `text`
# and `font`. Those belong to specific types, not to Item, so `property color
# color` on an Item is both legal and idiomatic -- TorIcon and TorWordmark
# declare exactly that on purpose. Listing them made this test reject correct
# code, which is the fastest way to get a test switched off.
RESERVED = {
    "activeFocus", "activeFocusOnTab", "anchors", "antialiasing", "baselineOffset",
    "children", "childrenRect", "clip", "containmentMask", "data", "enabled",
    "focus", "height", "implicitHeight", "implicitWidth", "layer", "opacity",
    "parent", "resources", "rotation", "scale", "smooth", "state", "states",
    "transform", "transformOrigin", "transitions", "visible", "visibleChildren",
    "width", "x", "y", "z",
}

# `property <type> <name>` — the declaration, not a binding.
DECL = re.compile(
    r"^\s*(?:readonly\s+|default\s+|required\s+)*property\s+"
    r"(?:var|int|real|double|bool|string|color|url|date|list<[^>]+>|[A-Z]\w*)\s+"
    r"(\w+)"
)
SIGNAL = re.compile(r"^\s*signal\s+(\w+)")


def check(path: Path) -> list[str]:
    """Walk the file tracking brace depth, so each object gets its own scope.

    Without scoping, two sibling Repeater delegates each declaring
    `required property var modelData` look like a duplicate declaration when
    they are two unrelated objects. Depth is a crude stand-in for a real QML
    parse, but it is enough to tell siblings apart.
    """
    problems: list[str] = []
    # depth -> {name: line}. Cleared for a depth as soon as we leave it, so a
    # later sibling at the same depth starts fresh.
    scopes: dict[int, dict[str, int]] = {}
    signals: dict[str, int] = {}
    depth = 0

    for lineno, line in enumerate(path.read_text().splitlines(), 1):
        stripped = line.split("//")[0]

        if (m := DECL.match(line)) is not None:
            name = m.group(1)
            if name in RESERVED:
                problems.append(
                    f"{path.name}:{lineno}: property '{name}' shadows the "
                    f"QQuickItem property of the same name"
                )
            here = scopes.setdefault(depth, {})
            if name in here:
                problems.append(
                    f"{path.name}:{lineno}: property '{name}' already declared "
                    f"in this object at line {here[name]}"
                )
            here[name] = lineno
        elif (m := SIGNAL.match(line)) is not None:
            signals[m.group(1)] = lineno

        opened, closed = stripped.count("{"), stripped.count("}")
        depth += opened
        for _ in range(closed):
            scopes.pop(depth, None)
            depth = max(0, depth - 1)

    # A property generates <name>Changed and a signal generates a <name>
    # handler slot, so the two namespaces overlap.
    declared = {n for names in scopes.values() for n in names}
    for name, lineno in signals.items():
        if name in declared:
            problems.append(
                f"{path.name}:{lineno}: signal '{name}' clashes with a property "
                f"of the same name"
            )

    return problems


def main() -> int:
    files = sorted(ROOT.glob("*.qml"))
    if not files:
        print("  no QML files found -- is this running from the repo root?")
        return 1

    problems: list[str] = []
    for f in files:
        problems.extend(check(f))

    if problems:
        for p in problems:
            print(f"  FAIL  {p}")
        return 1

    print(f"  ok    {len(files)} QML files declare no colliding names")
    return 0


if __name__ == "__main__":
    sys.exit(main())
