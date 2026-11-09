#!/usr/bin/env python3
"""Service.qml over time, under a real Qt engine.

    python3 tests/qml/scenarios.py --omarchy ~/src/omarchy

The decisions are tormarchyd's, and tests/daemon/scenarios.py holds them. What
is left in QML is the wiring, and this holds that: the daemon started once,
told who it serves, handed every click as a command; its views applied and
anything else ignored; a missing helper shown as not installed and tried again
later, quietly, rather than in a loop.
"""
import argparse
import json
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
FIXTURES = os.path.join(HERE, "fixtures")

ap = argparse.ArgumentParser()
ap.add_argument("--omarchy", default=os.environ.get("OMARCHY_PATH", "/usr/share/omarchy"))
ap.add_argument("--plugin", default=REPO)
args = ap.parse_args()

shell_dir = os.path.join(os.path.abspath(os.path.expanduser(args.omarchy)), "shell")
if not os.path.isdir(os.path.join(shell_dir, "Ui")):
    sys.exit(f"no Omarchy shell at {shell_dir} -- pass --omarchy <checkout>")
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PySide6.QtCore import QEventLoop, QMetaObject, Q_ARG, QTimer, QUrl  # noqa: E402
from PySide6.QtGui import QGuiApplication  # noqa: E402
from PySide6.QtQml import QQmlComponent, QQmlEngine  # noqa: E402
from PySide6 import QtQuick  # noqa: E402,F401

sys.path.insert(0, HERE)
import quickshell_stubs as stubs  # noqa: E402

imports = tempfile.mkdtemp(prefix="tormarchy-scenarios-")
shutil.copytree(os.path.join(shell_dir, "Commons"), os.path.join(imports, "qs", "Commons"))
shutil.copytree(os.path.join(shell_dir, "Ui"), os.path.join(imports, "qs", "Ui"))

app = QGuiApplication(sys.argv[:1])
engine = QQmlEngine()
engine.addImportPath(imports)
component = QQmlComponent(engine, QUrl.fromLocalFile(os.path.join(os.path.abspath(args.plugin), "Service.qml")))
if component.isError():
    sys.exit("Service.qml does not compile: " + "; ".join(e.toString() for e in component.errors()))

failures = 0


def check(ok, what):
    global failures
    print(("  ok    " if ok else "  FAIL  ") + what)
    if not ok:
        failures += 1


def wait(ms):
    loop = QEventLoop()
    QTimer.singleShot(ms, loop.quit)
    loop.exec()


def sent():
    return [json.loads(line) for _, line in stubs.STDIN_LOG]


def spawns():
    return [a for a in stubs.COMMAND_LOG if a and a[0].endswith("tormarchyd")]


def service(view=None, settings=None, live=True):
    if view:
        os.environ["TORMARCHY_STUB_VIEW"] = os.path.join(FIXTURES, view + ".json")
    else:
        os.environ.pop("TORMARCHY_STUB_VIEW", None)
    stubs.COMMAND_LOG.clear()
    stubs.STDIN_LOG.clear()
    svc = component.createWithInitialProperties({"settings": settings or {}, "live": live})
    if svc is None:
        sys.exit("Service.qml did not instantiate")
    wait(80)
    return svc


def view_of(svc):
    v = svc.property("view")
    return v.toVariant() if hasattr(v, "toVariant") else v


def call(svc, name, *a):
    QMetaObject.invokeMethod(svc, name, *[Q_ARG("QVariant", x) for x in a])


print("Starting the daemon")
svc = service("view-on", {"notifications": "all"})
check(spawns() == [["/usr/local/bin/tormarchyd"]], "it runs the installed daemon, by its fixed path, once")
first = sent()[0] if sent() else {}
check(first.get("cmd") == "hello" and first.get("settings", {}).get("notifications") == "all",
      "and first tells it the widget's settings")
check(first.get("pluginDir", "").endswith(os.path.basename(os.path.abspath(args.plugin))),
      "and where the plugin lives, so it can tell when setup is due")
view = view_of(svc)
check(svc.property("installed") is True and view.get("bar") == "routed", "its view is applied as it arrives")
svc.deleteLater()

print("Commands")
svc = service("view-on")
stubs.STDIN_LOG.clear()
call(svc, "toggle")
call(svc, "setMode", "strict")
call(svc, "setExit", "de")
call(svc, "setBridges", True)
call(svc, "watch", True)
call(svc, "watch", True)
call(svc, "watch", False)
cmds = sent()
check([c["cmd"] for c in cmds] == ["toggle", "setMode", "setExit", "setBridges", "watch", "watch", "watch"],
      "every click is a command, in order")
check(cmds[1].get("mode") == "strict" and cmds[2].get("cc") == "de" and cmds[3].get("on") is True,
      "carrying its argument")
check(svc.property("watchers") == 1, "two panels open and one closed is one still watching")
svc.setProperty("settings", {"mode": "strict"})
check(sent()[-1] == {"cmd": "settings", "settings": {"mode": "strict"}}, "a settings change is passed on")
svc.deleteLater()

print("Not installed")
svc = service(None)
check(svc.property("installed") is False and view_of(svc).get("statusText") == "Tor helpers are not installed",
      "a helper that is not there reads as not installed")
check(len(spawns()) == 1, "tried once")
wait(2300)
check(len(spawns()) == 2, "and again a little later, in case setup has run since")
wait(2300)
check(len(spawns()) == 2, "backing off each time, so a missing helper is not a loop")
svc.deleteLater()

print("An idle copy")
svc = service("view-on", live=False)
check(spawns() == [] and svc.property("installed") is False, "a widget's own copy runs nothing while the shared one exists")
svc.setProperty("live", True)
wait(80)
check(len(spawns()) == 1 and svc.property("installed") is True, "and starts the moment it is the only one")
svc.deleteLater()

shutil.rmtree(imports, ignore_errors=True)
print()
print("  the service wiring holds" if failures == 0 else f"  {failures} check(s) failed")
sys.exit(1 if failures else 0)
