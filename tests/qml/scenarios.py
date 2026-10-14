#!/usr/bin/env python3
"""Drive Service.qml through real sequences of states, under a real Qt engine.

    python3 tests/qml/scenarios.py --omarchy ~/src/omarchy

load.py proves the widget loads and renders; this proves what it *does* over
time -- which notifications a run of status reads produces, that a brief tor
restart is not mistaken for the kill switch, that bridge lines go to tormarchy
over stdin and never on a command line. Each scenario starts a fresh Service,
feeds it status files in order, and asserts on the commands the stubbed
Process recorded. Exit status is 0 only when every scenario passes.

Needs PySide6 and an Omarchy checkout, like load.py.
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

from PySide6.QtCore import QEventLoop, QMetaObject, QObject, QTimer, QUrl, SIGNAL  # noqa: E402
from PySide6.QtGui import QGuiApplication  # noqa: E402
from PySide6.QtQml import QQmlComponent, QQmlEngine  # noqa: E402
from PySide6 import QtQuick  # noqa: E402,F401

sys.path.insert(0, HERE)
import quickshell_stubs as stubs  # noqa: E402

imports = tempfile.mkdtemp(prefix="tormarchy-scenarios-")
shutil.copytree(os.path.join(shell_dir, "Commons"), os.path.join(imports, "qs", "Commons"))
shutil.copytree(os.path.join(shell_dir, "Ui"), os.path.join(imports, "qs", "Ui"))
status_file = os.path.join(imports, "status.json")
os.environ["TORMARCHY_STUB_STATUS"] = status_file

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


def fixture(name, **changes):
    data = json.load(open(os.path.join(FIXTURES, name + ".json")))
    data.update(changes)
    return data


def service(first, settings=None):
    """A fresh Service whose own first poll -- it refreshes once on start --
    reads `first`."""
    with open(status_file, "w") as f:
        json.dump(first, f)
    stubs.COMMAND_LOG.clear()
    stubs.STDIN_LOG.clear()
    svc = component.createWithInitialProperties({"settings": settings or {}})
    if svc is None:
        sys.exit("Service.qml did not instantiate")
    wait(80)
    return svc


def poll(svc, status):
    """One status read, answered with this status."""
    with open(status_file, "w") as f:
        json.dump(status, f)
    QMetaObject.invokeMethod(svc, "refresh")
    wait(80)


def toasts():
    return [argv for argv in stubs.COMMAND_LOG if argv[:1] == ["omarchy-notification-send"]]


def headline(argv):
    # The first argument after the options, as the sender itself parses it.
    i = argv.index("-p") + 1
    if argv[i] == "-r":
        i += 2
    return argv[i]


def pretend_blocked_for_a_while(svc):
    # "blocked" must outlast a tor restart before it counts; move its start back
    # rather than make the test sit through the real four seconds.
    svc.setProperty("_blockedSinceMs", svc.property("_blockedSinceMs") - 5000)


print("Notifications")

svc = service(fixture("on"))
poll(svc, fixture("blocked"))
check(toasts() == [], "a single blocked reading is not the kill switch yet (could be a tor restart)")
pretend_blocked_for_a_while(svc)
poll(svc, fixture("blocked"))
sent = toasts()
check(len(sent) == 1 and headline(sent[0]) == "Kill switch engaged" and "critical" in sent[0],
      "a blocked state that persists raises one critical toast")
check(all("notify-send" not in a for a in sent) and sent and sent[0][0] == "omarchy-notification-send",
      "it goes through omarchy-notification-send, never notify-send")
check(sent and sent[0][-4:] == ["--exec", "omarchy-shell", "tripleu.tor", "open"],
      "clicking it opens the panel")
poll(svc, fixture("on"))
sent = toasts()
check(len(sent) == 2 and headline(sent[1]) == "Tor is back" and sent[1][sent[1].index("-r") + 1] == "7",
      "Tor coming back replaces the kill-switch toast instead of stacking")
svc.deleteLater()

svc = service(fixture("on"))
poll(svc, fixture("off"))
check(toasts() == [], "an ordinary disconnect says nothing")
svc.deleteLater()

svc = service(fixture("on"))
poll(svc, fixture("blocked"))
poll(svc, fixture("on"))
check(toasts() == [], "a tor restart that recovers by the next poll says nothing")
svc.deleteLater()

svc = service(fixture("blocked"))
pretend_blocked_for_a_while(svc)
poll(svc, fixture("blocked"))
check(toasts() == [], "the first reading after the shell starts is a baseline, not a change")
svc.deleteLater()

svc = service(fixture("on"), {"notifications": "off"})
poll(svc, fixture("blocked"))
pretend_blocked_for_a_while(svc)
poll(svc, fixture("blocked"))
check(toasts() == [], "notifications set to off sends nothing, not even the kill switch")
svc.deleteLater()

svc = service(fixture("off"))
poll(svc, fixture("on"))
check(toasts() == [], "by default a successful connect is not announced")
svc.deleteLater()

svc = service(fixture("off"), {"notifications": "all"})
poll(svc, fixture("on"))
sent = toasts()
check(len(sent) == 1 and headline(sent[0]) == "Connected through Tor" and "Germany" in " ".join(sent[0]),
      "with notifications set to all, a connect is announced with its exit country")
svc.deleteLater()

os.environ["TORMARCHY_STUB_CONNECT_EXIT"] = "1"
svc = service(fixture("off"))
QMetaObject.invokeMethod(svc, "toggleConnected")
wait(150)
sent = toasts()
check(len(sent) == 1 and headline(sent[0]) == "Couldn't connect to Tor" and "stalled" in " ".join(sent[0]),
      "a connect that fails on its own raises a toast with the reason")
svc.deleteLater()
os.environ["TORMARCHY_STUB_CONNECT_EXIT"] = "126"
svc = service(fixture("off"))
QMetaObject.invokeMethod(svc, "toggleConnected")
wait(150)
check(toasts() == [], "dismissing the password prompt is not a failure worth a toast")
svc.deleteLater()
del os.environ["TORMARCHY_STUB_CONNECT_EXIT"]

print()
print("Polling")

svc = service(fixture("on"))
changes = {"path": 0, "hops": 0, "excludedExits": 0}
for name in changes:
    QObject.connect(svc, SIGNAL(name + "Changed()"), lambda n=name: changes.__setitem__(n, changes[n] + 1))
poll(svc, fixture("on"))
poll(svc, fixture("on"))
check(changes == {"path": 0, "hops": 0, "excludedExits": 0},
      "an unchanged circuit is not reassigned, so a hop's tooltip survives the poll")
poll(svc, fixture("on", path=["se", "ch", "nl"], excludedExits=["us"]))
check(changes["path"] == 1 and changes["excludedExits"] == 1, "a changed one is")
svc.deleteLater()

print()
print("Helper")

svc = service(fixture("on"))
svc.setProperty("bundledHelper", "/plugin/tormarchy")
wait(120)
check(svc.property("helperStale") is False, "a helper that matches the plugin's asks for nothing")
check(any("cmp -s" in " ".join(a) and "/plugin/tormarchy" in a for a in stubs.COMMAND_LOG),
      "it is compared with the helper this plugin ships")
svc.deleteLater()
os.environ["TORMARCHY_STUB_HELPER_STALE"] = "1"
svc = service(fixture("on"))
svc.setProperty("bundledHelper", "/plugin/tormarchy")
wait(120)
check(svc.property("helperStale") is True, "an installed helper older than the plugin is noticed")
svc.deleteLater()
os.environ["TORMARCHY_STUB_HELPER_STALE"] = "2"
svc = service(fixture("on"))
svc.setProperty("bundledHelper", "/plugin/tormarchy")
wait(120)
check(svc.property("helperStale") is False, "one that cannot be compared is not a reason to nag")
svc.deleteLater()
del os.environ["TORMARCHY_STUB_HELPER_STALE"]

print()
print("Bridges")

BRIDGE_LINES = open(os.path.join(FIXTURES, "bridges.txt")).read()
os.environ["TORMARCHY_STUB_CLIPBOARD"] = os.path.join(FIXTURES, "bridges.txt")


def ran(fragment):
    return [a for a in stubs.COMMAND_LOG if fragment in " ".join(a)]


def lines_in_argv():
    # Any one bridge's address or fingerprint on any command line is a leak:
    # argv is world-readable in /proc.
    needles = [line.split()[1] for line in BRIDGE_LINES.splitlines() if line.strip()]
    needles += [line.split()[2] for line in BRIDGE_LINES.splitlines() if line.strip()]
    return [a for a in stubs.COMMAND_LOG if any(n in " ".join(a) for n in needles)]


def paste(svc):
    QMetaObject.invokeMethod(svc, "pasteBridges")
    wait(150)


os.environ["TORMARCHY_STUB_BRIDGE_CHECK"] = os.path.join(FIXTURES, "bridge-check-ok.json")
svc = service(fixture("on"))
paste(svc)
read = ran("wl-paste")
check(read and read[0][:2] == ["bash", "-c"] and "--type text/plain" in read[0][2]
      and "head -c" in read[0][2] and read[0][3:5] == ["paste-bridges", "65536"],
      "a paste reads the clipboard as plain text, capped at the helper's limit")
checked = [data for cmd, data in stubs.STDIN_LOG if "bridge check" in cmd]
check(len(checked) == 1 and checked[0].strip() == BRIDGE_LINES.strip(),
      "the pasted lines reach `bridge check` over stdin, whole")
preview = svc.property("bridgePreview")
preview = preview.toVariant() if hasattr(preview, "toVariant") else preview
check(isinstance(preview, dict) and preview.get("count") == 3 and preview.get("invalidLines") == 0,
      "the check's answer becomes the preview")
QMetaObject.invokeMethod(svc, "applyBridges")
wait(150)
applied = [data for cmd, data in stubs.STDIN_LOG if "bridge set" in cmd]
check(len(applied) == 1 and applied[0].strip() == BRIDGE_LINES.strip(),
      "Use these sends the same lines to `bridge set` over stdin")
check(ran("bridge set") and ran("bridge set")[0] == ["bash", "-c", "exec tormarchy bridge set"],
      "`bridge set` takes no arguments that could carry a line")
check(lines_in_argv() == [], "no bridge address or fingerprint appears on any command line")
check(svc.property("_pendingBridgeText") == "" and not svc.property("bridgePreview"),
      "once applied, the pasted text is gone from memory")
check(ran("bridge status --json") != [], "applying reloads what the bridge tile shows")
check(toasts() == [], "switching bridges on yourself is not announced")
svc.deleteLater()

os.environ["TORMARCHY_STUB_BRIDGE_CHECK"] = os.path.join(FIXTURES, "bridge-check-missing.json")
svc = service(fixture("on"))
paste(svc)
QMetaObject.invokeMethod(svc, "applyBridges")
wait(150)
check(ran("bridge set") == [], "lines needing a transport this machine lacks are never applied")
svc.deleteLater()

os.environ["TORMARCHY_STUB_BRIDGE_CHECK"] = os.path.join(FIXTURES, "bridge-check-invalid.json")
svc = service(fixture("on"))
paste(svc)
QMetaObject.invokeMethod(svc, "applyBridges")
wait(150)
check(ran("bridge set") == [], "a paste with lines that are not bridge lines is never applied")
svc.deleteLater()

os.environ["TORMARCHY_STUB_BRIDGE_CHECK"] = os.path.join(FIXTURES, "bridge-check-ok.json")
svc = service(fixture("on"))
paste(svc)
QMetaObject.invokeMethod(svc, "cancelBridges")
check(svc.property("_pendingBridgeText") == "" and not svc.property("bridgePreview"),
      "Cancel drops the pasted text")
svc.setProperty("watching", True)
paste(svc)
svc.setProperty("watching", False)
check(svc.property("_pendingBridgeText") == "", "closing the panel drops the pasted text")
svc.deleteLater()

empty = os.path.join(imports, "empty.txt")
open(empty, "w").close()
os.environ["TORMARCHY_STUB_CLIPBOARD"] = empty
svc = service(fixture("on"))
paste(svc)
check(ran("bridge check") == [] and svc.property("bridgeMessage") != "",
      "an empty clipboard says so, and runs nothing")
svc.deleteLater()
# More than the limit: the capped read refuses it, and nothing goes further.
os.environ["TORMARCHY_STUB_CLIPBOARD"] = os.path.join(FIXTURES, "bridges.txt")
for code, what in ((3, "too much on the clipboard"), (4, "a clipboard that will not answer")):
    os.environ["TORMARCHY_STUB_CLIPBOARD_EXIT"] = str(code)
    before = len(ran("bridge check"))
    svc = service(fixture("on"))
    paste(svc)
    check(len(ran("bridge check")) == before and svc.property("bridgeMessage") != ""
          and svc.property("_pendingBridgeText") == "" and not svc.property("bridgePreview"),
          f"{what} says so, holds nothing, and runs nothing")
    svc.deleteLater()
del os.environ["TORMARCHY_STUB_CLIPBOARD_EXIT"]
for name in ("TORMARCHY_STUB_CLIPBOARD", "TORMARCHY_STUB_BRIDGE_CHECK"):
    del os.environ[name]

print()
shutil.rmtree(imports, ignore_errors=True)
if failures:
    print(f"  {failures} scenario check(s) failed")
    sys.exit(1)
print("  all scenarios passed")
