#!/usr/bin/env python3
"""Load the bar widget the way Omarchy's shell does, under a real Qt engine.

    python3 tests/qml/load.py --omarchy ~/src/omarchy
    python3 tests/qml/load.py --omarchy /usr/share/omarchy --status tests/qml/fixtures/on.json \\
        --bar bar.png --panel panel.png

The shell loads a third-party widget with Qt.createComponent and, if that
fails, logs one line and leaves the bar without it. Nothing else says so: the
plugin installs, enables, and simply never appears. That is the failure this
reproduces off the machine. It compiles Panel.qml against Omarchy's own
qs.Commons and qs.Ui (copied from the checkout you point it at, so it tests
against that version), injects a PluginBarApi as `bar` the way the bar does,
and prints every error and warning the plugin's QML raises. Exit status is 0
only when there are none.

Needs PySide6 (pip install PySide6) and an Omarchy checkout or install. The
Quickshell types are stood in for by quickshell_stubs.py.
"""
import argparse
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))

ap = argparse.ArgumentParser()
ap.add_argument("--omarchy", default=os.environ.get("OMARCHY_PATH", "/usr/share/omarchy"),
                help="Omarchy checkout or install whose shell/ to load against")
ap.add_argument("--plugin", default=REPO)
ap.add_argument("--status", help="JSON to answer `tormarchy status --json` with; without it, "
                                 "the helper reads as not installed")
ap.add_argument("--bar", help="save the bar slot to this PNG")
ap.add_argument("--panel", help="open the panel and save it to this PNG")
ap.add_argument("--foreground", default="#cdd6f4")
ap.add_argument("--background", default="#1e1e2e")
ap.add_argument("--settle", type=int, default=1500, help="milliseconds to run before reporting")
ap.add_argument("--doctor", help="JSON to answer `tormarchy doctor --json` with")
ap.add_argument("--bridges", help="JSON to answer `tormarchy bridge status --json` with")
ap.add_argument("--bridge-check", help="JSON to answer `tormarchy bridge check` with")
ap.add_argument("--clipboard", help="file whose contents `wl-paste` returns")
ap.add_argument("--expand", choices=["leaks", "bridges"], help="open this tool beneath the tools band")
ap.add_argument("--call", action="append", default=[],
                help="after opening, call this function on the panel root, e.g. --call openLeakCheck; "
                     "tor.<name> calls it on the service")
ap.add_argument("--font", action="append", default=[], help="extra font file to load, e.g. a Nerd Font, so glyphs render")
ap.add_argument("--log", action="store_true", help="print every command run and every stdin payload")
args = ap.parse_args()

shell_dir = os.path.join(os.path.abspath(os.path.expanduser(args.omarchy)), "shell")
if not os.path.isdir(os.path.join(shell_dir, "Ui")):
    sys.exit(f"no Omarchy shell at {shell_dir} -- pass --omarchy <checkout>")
for value, name in ((args.status, "TORMARCHY_STUB_STATUS"), (args.doctor, "TORMARCHY_STUB_DOCTOR"),
                    (args.bridges, "TORMARCHY_STUB_BRIDGES"), (args.bridge_check, "TORMARCHY_STUB_BRIDGE_CHECK"),
                    (args.clipboard, "TORMARCHY_STUB_CLIPBOARD")):
    if value:
        os.environ[name] = os.path.abspath(value)

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QMetaObject, QObject, QTimer, QUrl, QtMsgType, qInstallMessageHandler  # noqa: E402
from PySide6.QtGui import QFontDatabase, QGuiApplication  # noqa: E402
from PySide6.QtQml import QQmlComponent, QQmlEngine  # noqa: E402
# Imported for its wrappers: without it the window and items come back as bare
# QWindow and QObject, with no grabWindow() or setParentItem().
from PySide6 import QtQuick  # noqa: E402,F401

sys.path.insert(0, HERE)
import quickshell_stubs  # noqa: E402,F401  (registers the Quickshell modules)

# qs.Commons and qs.Ui, laid out as the module paths the plugin imports.
imports = tempfile.mkdtemp(prefix="tormarchy-qml-")
shutil.copytree(os.path.join(shell_dir, "Commons"), os.path.join(imports, "qs", "Commons"))
shutil.copytree(os.path.join(shell_dir, "Ui"), os.path.join(imports, "qs", "Ui"))

messages = []
qInstallMessageHandler(lambda mode, ctx, msg: messages.append((mode, msg)))

app = QGuiApplication(sys.argv[:1])
for font in args.font:
    if QFontDatabase.addApplicationFont(os.path.abspath(font)) < 0:
        print("note  could not load font", font)
engine = QQmlEngine()
engine.addImportPath(imports)

panel_mode = bool(args.panel)
host_qml = """
import QtQuick
import qs.Ui

Window {
  id: win
  width: %(w)d
  height: %(h)d
  visible: true
  color: "%(bg)s"

  property var widget: null
  property string error: ""

  // What Bar.qml hands an installed third-party widget as `bar`.
  PluginBarApi {
    id: api
    pluginId: "tripleu.tor"
    moduleName: "tripleu.tor"
    foreground: "%(fg)s"
    barForeground: "%(fg)s"
    background: "%(bg)s"
    urgent: "#f38ba8"
    fontFamily: "monospace"
    barSize: 26
  }

  // The KeyboardPanel draws on a layer-shell surface of its own, which the
  // stubbed PanelWindow cannot show; its content is moved here to be seen.
  Rectangle {
    objectName: "card"
    x: 20
    y: 50
    width: 0
    height: 0
    color: "%(bg)s"
    border.color: Qt.rgba(1, 1, 1, 0.25)
    radius: 6
    Item { objectName: "cardContent"; anchors.fill: parent; anchors.margins: 14 }
  }

  Item {
    objectName: "slot"
    x: %(slotx)s
    y: %(sloty)s
    height: 26
    width: loader.item ? loader.item.implicitWidth : 0

    Loader {
      id: loader
      anchors.fill: parent
      // The slot's injectProps.
      onLoaded: {
        if ("bar" in item) item.bar = api
        if ("moduleName" in item) item.moduleName = "tripleu.tor"
        if ("settings" in item) item.settings = ({ mode: "lan", refreshIntervalSec: 5, speedIntervalSec: 3 })
        win.widget = item
      }
    }
  }

  // The shell's loadPluginWidget, synchronously.
  function load(url) {
    var comp = Qt.createComponent(url, Component.PreferSynchronous)
    if (comp.status === Component.Error) { error = comp.errorString(); return false }
    loader.sourceComponent = comp
    return true
  }
}
""" % {
    "fg": args.foreground, "bg": args.background,
    "w": 520 if panel_mode else 200, "h": 720 if panel_mode else 40,
    "slotx": "120" if panel_mode else "Math.round((win.width - width) / 2)",
    "sloty": "10" if panel_mode else "7",
}

host = QQmlComponent(engine)
host.setData(host_qml.encode(), QUrl.fromLocalFile(os.path.join(imports, "host.qml")))
win = host.create()
if win is None:
    for e in host.errors():
        print("host:", e.toString())
    sys.exit(2)

failed = False
if not win.load(QUrl.fromLocalFile(os.path.join(os.path.abspath(args.plugin), "Panel.qml"))):
    print("FAIL  Panel.qml does not compile. The shell would skip it and nothing would appear in the bar:")
    print("      " + str(win.property("error")).strip().replace("\n", "\n      "))
    failed = True


def find(root, prefix):
    for o in root.findChildren(QObject):
        if o.metaObject().className().startswith(prefix):
            return o
    return None


def mount_panel():
    widget = win.property("widget")
    if widget is None:
        return
    widget.open()
    catcher = find(widget, "PanelKeyCatcher")
    keyboard_panel = find(widget, "KeyboardPanel")
    if catcher is None or keyboard_panel is None:
        print("note  could not find the panel content to render")
        return
    card = win.findChild(QObject, "card")
    card.setProperty("width", keyboard_panel.property("contentWidth") + 28)
    catcher.setParentItem(win.findChild(QObject, "cardContent"))
    if args.expand:
        widget.setProperty("expandedTool", args.expand)
    QTimer.singleShot(50, lambda: call_functions(widget))


def call_functions(widget):
    service = find(widget, "Service")
    for name in args.call:
        target, method = (service, name[4:]) if name.startswith("tor.") else (widget, name)
        if target is None or not QMetaObject.invokeMethod(target, method):
            print("note  could not call", name)


def fit_panel():
    widget = win.property("widget")
    flick = find(widget, "QQuickFlickable") if widget is not None else None
    if flick is not None:
        win.findChild(QObject, "card").setProperty("height", flick.property("contentHeight") + 28)


def finish():
    try:
        report()
    except Exception as e:  # never leave the event loop running on a harness bug
        print("FAIL  harness error:", e)
        app.exit(2)


def report():
    global failed
    if not failed and win.property("widget") is None:
        print("FAIL  Panel.qml compiled but did not instantiate")
        failed = True
    if args.bar or args.panel:
        image = win.grabWindow()
        if args.panel:
            image.save(args.panel)
            print("saved", args.panel)
        if args.bar:
            slot = win.findChild(QObject, "slot")
            image.copy(int(slot.property("x")) - 10, int(slot.property("y")) - 4,
                       int(slot.property("width")) + 20, 34).save(args.bar)
            print("saved", args.bar)

    plugin_dir = os.path.abspath(args.plugin)
    reported = 0
    for mode, msg in messages:
        # Only what the plugin's own files raise. The stubbed host makes noise of
        # its own (no theme files, no screens) that says nothing about the plugin.
        if plugin_dir not in msg:
            continue
        level = "FAIL" if mode in (QtMsgType.QtWarningMsg, QtMsgType.QtCriticalMsg, QtMsgType.QtFatalMsg) else "log "
        print(f"{level}  {msg}")
        if level == "FAIL":
            failed = True
        reported += 1
    if args.log:
        for argv in quickshell_stubs.COMMAND_LOG:
            print("ran  ", argv)
        for cmd, data in quickshell_stubs.STDIN_LOG:
            print("stdin", repr(cmd), "<-", repr(data[:80]))
    print("result: " + ("FAIL" if failed else "ok") + f" ({reported} message(s) from the plugin)")
    shutil.rmtree(imports, ignore_errors=True)
    app.exit(1 if failed else 0)


if panel_mode and not failed:
    QTimer.singleShot(300, mount_panel)
    QTimer.singleShot(max(600, args.settle - 300), fit_panel)
QTimer.singleShot(args.settle, finish)
sys.exit(app.exec())
