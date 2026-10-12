"""Just enough of Quickshell, registered from Python, for Omarchy's own qs.Commons
and qs.Ui -- and this plugin on top of them -- to compile and run under a stock
Qt QML engine from PySide6. Nothing here talks to Wayland or runs anything.

Shapes only: every type declares the properties and signals the shell's QML
assigns to, because assigning to one that does not exist is a compile error,
and a compile error is exactly what load.py is there to find. When Omarchy's
components start using something new, load.py reports it as a host error and
the property goes here.

Process is the one stand-in with behaviour. It answers `tormarchy status --json`,
`doctor --json`, `bridge status --json`, `bridge check` and `wl-paste` from the
fixture files named by the TORMARCHY_STUB_* variables, streams a fixed latency
for `pingd`, and exits 0 for anything else. A process with stdin enabled waits
for stdin to be closed before it "runs", as one reading to EOF would. Every argv
and every stdin payload is logged in COMMAND_LOG and STDIN_LOG.
"""
import os

from PySide6.QtCore import QObject, Property, Signal, Slot, ClassInfo, QEnum, QTimer
from PySide6.QtGui import QColor
from PySide6.QtQml import (QmlElement, QmlSingleton, QmlAttached, QmlAnonymous,
                           ListProperty, QmlUncreatable)
from enum import IntEnum

QML_IMPORT_MAJOR_VERSION = 1
QML_IMPORT_MINOR_VERSION = 0


def prop(typ, name, default=None):
    """A plain read/write Qt property with a change signal."""
    attr = "_" + name

    def getter(self):
        return getattr(self, attr, default)

    def setter(self, value):
        if getattr(self, attr, default) != value:
            setattr(self, attr, value)
            getattr(self, name + "Changed").emit()

    return getter, setter


def make_class(base, clsname, props, signals=(), extra=None):
    ns = {}
    for name, typ, default in props:
        ns[name + "Changed"] = Signal()
    for sig in signals:
        if isinstance(sig, tuple):
            ns[sig[0]] = Signal(*sig[1:])
        else:
            ns[sig] = Signal()
    for name, typ, default in props:
        g, s = prop(typ, name, default)
        ns[name] = Property(typ, g, s, notify=ns[name + "Changed"])
    if extra:
        ns.update(extra)
    return type(clsname, (base,), ns)


def object_list(clsname, base=QObject, props=(), signals=(), extra=None):
    """A QObject with a default `data` list property, so QML children nest."""

    def _kids(self):
        if not hasattr(self, "_children"):
            self._children = []
        return self._children

    ns = dict(extra or {})
    ns["data"] = ListProperty(QObject,
                              append=lambda self, o: _kids(self).append(o),
                              at=lambda self, i: _kids(self)[i],
                              count=lambda self: len(_kids(self)),
                              clear=lambda self: _kids(self).clear())
    cls = make_class(base, clsname, props, signals, ns)
    return ClassInfo(DefaultProperty="data")(cls)


# ------------------------------------------------------------------ Quickshell

QML_IMPORT_NAME = "Quickshell"


@QmlElement
@QmlSingleton
class Quickshell(QObject):
    screensChanged = Signal()

    @Slot(str, result=str)
    def env(self, name):
        return os.environ.get(name, "")

    @Slot("QVariant")
    def execDetached(self, cmd):
        pass

    def _screens(self):
        return []

    screens = Property("QVariantList", _screens, notify=screensChanged)

    def _shell_dir(self):
        return ""

    shellDir = Property(str, _shell_dir, constant=True)


class QsWindowAttached(QObject):
    windowChanged = Signal()
    maskChanged = Signal()

    def _w(self):
        return None

    window = Property(QObject, _w, notify=windowChanged)


@QmlElement
@QmlUncreatable("attached only")
@QmlAttached(QsWindowAttached)
class QsWindow(QObject):
    @staticmethod
    def qmlAttachedProperties(self, o):
        return QsWindowAttached(o)


Singleton = QmlElement(object_list("Singleton"))
Scope = QmlElement(object_list("Scope"))
ShellRoot = QmlElement(object_list("ShellRoot"))

Anchors = QmlAnonymous(make_class(QObject, "WindowAnchors", [
    ("top", bool, False), ("bottom", bool, False), ("left", bool, False), ("right", bool, False)]))
Margins = QmlAnonymous(make_class(QObject, "WindowMargins", [
    ("top", int, 0), ("bottom", int, 0), ("left", int, 0), ("right", int, 0)]))

_window_props = [
    ("screen", "QVariant", None), ("visible", bool, True), ("color", QColor, QColor("white")),
    ("implicitWidth", float, 0.0), ("implicitHeight", float, 0.0),
    ("width", float, 0.0), ("height", float, 0.0),
    ("exclusiveZone", int, 0), ("exclusionMode", int, 0), ("focusable", bool, False),
    ("mask", QObject, None), ("surfaceFormat", "QVariant", None),
    ("contentItem", QObject, None), ("aboveWindows", bool, True),
    ("backingWindowVisible", bool, False), ("devicePixelRatio", float, 1.0),
    ("windowTransform", "QVariant", None),
]


def _window(name):
    def anchors(self):
        if not hasattr(self, "_anchors"):
            self._anchors = Anchors()
        return self._anchors

    def margins(self):
        if not hasattr(self, "_margins"):
            self._margins = Margins()
        return self._margins

    return object_list(name, props=_window_props, signals=["windowConnected"], extra={
        "anchors": Property(Anchors, anchors, constant=True),
        "margins": Property(Margins, margins, constant=True),
    })


PanelWindow = QmlElement(_window("PanelWindow"))
FloatingWindow = QmlElement(_window("FloatingWindow"))


PopupAnchor = QmlAnonymous(make_class(QObject, "PopupAnchor", [
    ("item", QObject, None), ("window", QObject, None), ("rect", "QVariant", None),
    ("edges", int, 0), ("gravity", int, 0), ("adjustment", int, 0)]))


def _popup_anchor(self):
    if not hasattr(self, "_anchor"):
        self._anchor = PopupAnchor()
    return self._anchor


PopupWindow = QmlElement(object_list("PopupWindow", props=_window_props, extra={
    "anchor": Property(PopupAnchor, _popup_anchor, constant=True)}))

Region = QmlElement(object_list("Region", props=[
    ("item", QObject, None), ("x", int, 0), ("y", int, 0), ("width", int, 0), ("height", int, 0),
    ("intersection", int, 0), ("shape", int, 0), ("radius", int, 0)],
    extra={"regions": ListProperty(QObject, append=lambda self, o: None)}))

TransformWatcher = QmlElement(make_class(QObject, "TransformWatcher", [
    ("a", QObject, None), ("b", QObject, None), ("commonParent", QObject, None),
    ("transform", "QVariant", None)]))

Variants = QmlElement(object_list("Variants", props=[
    ("model", "QVariant", None), ("delegate", "QQmlComponent*", None)]))
LazyLoader = QmlElement(object_list("LazyLoader", props=[
    ("active", bool, False), ("loading", bool, False), ("component", "QQmlComponent*", None),
    ("source", str, ""), ("item", QObject, None)]))


# ------------------------------------------------------------ Quickshell.Io

QML_IMPORT_NAME = "Quickshell.Io"


# Every command a Process ran, as argv, and everything written to its stdin.
# load.py reads these to assert on what the plugin asked for -- that bridge
# lines went over stdin and not argv, that a notification used the right sender.
COMMAND_LOG = []
STDIN_LOG = []


def _fixture(name):
    path = os.environ.get(name, "")
    return open(path).read() if path and os.path.exists(path) else None


def _proc_get_running(self):
    return getattr(self, "_running", False)


def _proc_set_running(self, v):
    if v == _proc_get_running(self):
        return
    self._running = v
    self.runningChanged.emit()
    if v:
        self._stdin = ""
        QTimer.singleShot(0, lambda: _proc_start(self))


def _proc_start(self):
    if not _proc_get_running(self):
        return
    self.started.emit()
    # With stdin open the process waits for its input to end, as a real one
    # reading to EOF would; closing stdin (stdinEnabled = false) is the EOF.
    if not _proc_get_stdin(self):
        _proc_run(self)


def _proc_get_stdin(self):
    return getattr(self, "_stdinEnabled", False)


def _proc_set_stdin(self, v):
    if v == _proc_get_stdin(self):
        return
    self._stdinEnabled = v
    self.stdinEnabledChanged.emit()
    if not v and _proc_get_running(self):
        QTimer.singleShot(0, lambda: _proc_run(self))


def _proc_write(self, data):
    self._stdin = getattr(self, "_stdin", "") + str(data)


def _proc_finish(self, code):
    self._running = False
    self.runningChanged.emit()
    self.exited.emit(code, 0)


def _proc_run(self):
    if not _proc_get_running(self):
        return
    argv = [str(c) for c in (getattr(self, "_command", []) or [])]
    # Matched with the shell quoting run() adds taken off, so
    # bash -c "exec 'tormarchy' 'connect' 'lan'" reads as tormarchy connect lan.
    cmd = " ".join(argv).replace("'", "")
    COMMAND_LOG.append(argv)
    if getattr(self, "_stdin", ""):
        STDIN_LOG.append((cmd, self._stdin))
    out = getattr(self, "_stdout", None)
    if "pingd" in cmd:
        if out is not None and hasattr(out, "read"):
            out.read.emit(os.environ.get("TORMARCHY_STUB_PING", "612"))
        return  # keeps running, like the real stream
    text, code = "", 0
    if "status --json" in cmd and "bridge" not in cmd:
        text = _fixture("TORMARCHY_STUB_STATUS")
        if text is None:
            _proc_finish(self, 127)
            return
    elif "doctor --json" in cmd:
        text = _fixture("TORMARCHY_STUB_DOCTOR") or ""
        code = 0 if '"ok":true' in text.replace(" ", "") else 1
    elif "bridge status --json" in cmd:
        text = _fixture("TORMARCHY_STUB_BRIDGES") or '{"state":"off","count":0,"transports":[]}'
    elif "bridge check" in cmd:
        text = _fixture("TORMARCHY_STUB_BRIDGE_CHECK") or ""
    elif argv[:1] == ["wl-paste"]:
        text = _fixture("TORMARCHY_STUB_CLIPBOARD") or ""
    elif "cmp -s" in cmd:
        # The installed helper against the bundled one: 1 when they differ.
        code = int(os.environ.get("TORMARCHY_STUB_HELPER_STALE", "0"))
    elif argv[:1] == ["omarchy-notification-send"]:
        text = "7\n" if "-p" in argv else ""
    elif "exit --list" in cmd:
        text = "de\nnl\nus\nse\nch\n"
    elif "tormarchy connect" in cmd and os.environ.get("TORMARCHY_STUB_CONNECT_EXIT"):
        # A connect that fails the way the real one does: a message on stderr.
        code = int(os.environ["TORMARCHY_STUB_CONNECT_EXIT"])
        err = getattr(self, "_stderr", None)
        if err is not None and hasattr(err, "streamFinished"):
            err.setProperty("text", os.environ.get("TORMARCHY_STUB_CONNECT_ERROR", "tormarchy: tor stalled at 40% bootstrap."))
            err.streamFinished.emit()
    if out is not None and hasattr(out, "streamFinished"):
        out.setProperty("text", text)
        out.streamFinished.emit()
    _proc_finish(self, code)


_running_changed = Signal()
_stdin_changed = Signal()
Process = QmlElement(object_list("Process", props=[
    ("command", "QVariantList", []), ("environment", "QVariantMap", {}),
    ("clearEnvironment", bool, False), ("workingDirectory", str, ""),
    ("stdout", QObject, None), ("stderr", QObject, None),
    ("processId", "QVariant", None)],
    signals=[("exited", int, int), "started"],
    extra={
        "runningChanged": _running_changed,
        "running": Property(bool, _proc_get_running, _proc_set_running, notify=_running_changed),
        "stdinEnabledChanged": _stdin_changed,
        "stdinEnabled": Property(bool, _proc_get_stdin, _proc_set_stdin, notify=_stdin_changed),
        # Named explicitly: a Slot otherwise takes the function's own name, and
        # QML would find `_proc_write`, not `write`.
        "startDetached": Slot(name="startDetached")(lambda self: None),
        "signal": Slot(int, name="signal")(lambda self, s: None),
        "write": Slot(str, name="write")(_proc_write),
    }))


StdioCollector = QmlElement(make_class(QObject, "StdioCollector", [
    ("text", str, ""), ("data", "QVariant", None), ("waitForEnd", bool, True)],
    signals=["streamFinished"]))

SplitParser = QmlElement(make_class(QObject, "SplitParser", [("splitMarker", str, "\n")],
                                    signals=[("read", str)]))


@QmlElement
class IpcHandler(QObject):
    targetChanged = Signal()
    enabledChanged = Signal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._target = ""
        self._enabled = True

    def _get_target(self):
        return self._target

    def _set_target(self, v):
        self._target = v
        self.targetChanged.emit()

    def _get_enabled(self):
        return self._enabled

    def _set_enabled(self, v):
        self._enabled = v
        self.enabledChanged.emit()

    target = Property(str, _get_target, _set_target, notify=targetChanged)
    enabled = Property(bool, _get_enabled, _set_enabled, notify=enabledChanged)


# Reads nothing unless TORMARCHY_STUB_FILES is set: the tests describe a machine
# with no Omarchy state at all. load.py --theme sets it, with HOME pointed at a
# theme of its own, so Omarchy's Color reads a real colors.toml.
def _fv_get_path(self):
    return getattr(self, "_path", "")


def _fv_set_path(self, v):
    if v == _fv_get_path(self):
        return
    self._path = v
    self.pathChanged.emit()
    if os.environ.get("TORMARCHY_STUB_FILES"):
        QTimer.singleShot(0, lambda: _fv_load(self))


def _fv_load(self):
    try:
        with open(_fv_get_path(self), encoding="utf-8") as f:
            self._text = f.read()
    except OSError:
        self._text = ""
        self.loadFailed.emit()
        return
    self.loaded.emit()


_fv_path_changed = Signal()
FileView = QmlElement(object_list("FileView", props=[
    ("watchChanges", bool, False), ("blockLoading", bool, False),
    ("blockAllReads", bool, False), ("printErrors", bool, True), ("preload", bool, True),
    ("atomicWrites", bool, True), ("loaded_", bool, False), ("adapter", QObject, None)],
    signals=["loaded", "loadFailed", "fileChanged", "saved", "saveFailed"],
    extra={
        "pathChanged": _fv_path_changed,
        "path": Property(str, _fv_get_path, _fv_set_path, notify=_fv_path_changed),
        # Named explicitly, like Process.write: a Slot otherwise takes the
        # function's own name and QML finds no text().
        "text": Slot(result=str, name="text")(lambda self: getattr(self, "_text", "")),
        "reload": Slot(name="reload")(lambda self: _fv_load(self) if os.environ.get("TORMARCHY_STUB_FILES") else None),
        "setText": Slot(str, name="setText")(lambda self, t: None),
        "writeAdapter": Slot(name="writeAdapter")(lambda self: None),
    }))
JsonAdapter = QmlElement(object_list("JsonAdapter"))


# -------------------------------------------------------- Quickshell.Wayland

QML_IMPORT_NAME = "Quickshell.Wayland"


@QmlElement
class WlrLayer(QObject):
    class Layer(IntEnum):
        Background = 0
        Bottom = 1
        Top = 2
        Overlay = 3
    QEnum(Layer)


@QmlElement
class WlrKeyboardFocus(QObject):
    Focus = IntEnum("Focus", [("None", 0), ("Exclusive", 1), ("OnDemand", 2)])
    QEnum(Focus)


WlrLayershellAttached = make_class(QObject, "WlrLayershellAttached", [
    ("namespace", str, ""), ("layer", int, 2), ("keyboardFocus", int, 0),
    ("exclusionMode", int, 0)])


@QmlElement
@QmlUncreatable("attached only")
@QmlAttached(WlrLayershellAttached)
class WlrLayershell(QObject):
    @staticmethod
    def qmlAttachedProperties(self, o):
        return WlrLayershellAttached(o)


# ------------------------------------------------------- Quickshell.Hyprland

QML_IMPORT_NAME = "Quickshell.Hyprland"


@QmlElement
@QmlSingleton
class Hyprland(QObject):
    focusedMonitorChanged = Signal()
    monitorsChanged = Signal()

    @Slot()
    def refreshMonitors(self):
        pass

    @Slot(str)
    def dispatch(self, s):
        pass

    def _none(self):
        return None

    focusedMonitor = Property(QObject, _none, notify=focusedMonitorChanged)
    monitors = Property(QObject, _none, notify=monitorsChanged)


HyprlandFocusGrab = QmlElement(make_class(QObject, "HyprlandFocusGrab", [
    ("active", bool, False), ("windows", "QVariantList", [])], signals=["cleared"]))


# --------------------------------------------- Quickshell.Services.Pipewire

QML_IMPORT_NAME = "Quickshell.Services.Pipewire"


@QmlElement
@QmlSingleton
class Pipewire(QObject):
    changed = Signal()

    def _none(self):
        return None

    defaultAudioSource = Property(QObject, _none, notify=changed)
    defaultAudioSink = Property(QObject, _none, notify=changed)
    ready = Property(bool, lambda self: False, notify=changed)
