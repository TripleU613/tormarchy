import QtQuick
import Quickshell.Io

// The plugin's service: one tormarchyd for every monitor's widget. Everything
// the panel shows is decided in the daemon and arrives here as one JSON view
// per change; everything the panel asks for goes back as one JSON command. No
// state machine lives in QML any more -- only the process, its lines, and a
// restart if it ever exits.
Item {
  id: root

  property var settings: ({})

  // The shell runs one of these for the plugin and every monitor's widget
  // shares it. A widget also carries its own, for a bar that will not hand it
  // the shared one; that copy is not live while the shared one exists, and an
  // idle copy runs nothing.
  property bool live: true

  // The daemon's view, or the one shown before it has spoken: also what a
  // machine without the helper installed sees.
  readonly property var placeholder: ({
    installed: false, helperStale: false, statusText: "Tor helpers are not installed", bar: "off",
    active: false, busy: false, canToggle: false, switchCaption: "OFF", switchCaptionLit: false,
    mode: "lan", modes: [], exitCaption: "EXIT", exitValue: "auto", exitOptions: [],
    warning: { short: "", full: "" },
    latency: { text: "— ms", fraction: 0, measuring: false, caption: "", captionError: false },
    canNewCircuit: false,
    stats: { visible: false, hops: [], traffic: "", age: "" },
    leak: { glyph: "", failed: false, subtitle: "", subtitleError: false, checking: false, hasResult: false, checks: [] },
    bridges: { state: "off", subtitle: "Off", hasPreview: false, previewText: "", chips: [], invalidLabel: "",
               message: { short: "", full: "" }, canApply: false, working: false, toggleLabel: "" },
    error: { short: "", full: "" }, actionStatus: ""
  })
  property var view: placeholder
  readonly property bool installed: view.installed === true

  // Our own directory, for the daemon to compare its version with the plugin's.
  readonly property string pluginDir: String(Qt.resolvedUrl(".")).replace(/^file:\/\//, "").replace(/\/$/, "")

  // Counted, because every monitor's panel shares this service: one closing
  // must not stop what another is still showing.
  property int watchers: 0
  function watch(on) {
    watchers = Math.max(0, watchers + (on ? 1 : -1))
    send({ cmd: "watchers", count: watchers })
    // Opening the panel is also when a helper installed since is noticed.
    if (on && live && !daemon.running) restart.restart()
  }

  function send(command) {
    if (!started) return
    daemon.write(JSON.stringify(command) + "\n")
  }

  function toggle() { send({ cmd: "toggle" }) }
  function connect() { send({ cmd: "connect" }) }
  function disconnect() { send({ cmd: "disconnect" }) }
  function setMode(mode) { send({ cmd: "setMode", mode: String(mode) }) }
  function newCircuit() { send({ cmd: "newCircuit" }) }
  function setExit(code) { send({ cmd: "setExit", cc: String(code || "auto") }) }
  function loadExits() { send({ cmd: "loadExits" }) }
  function measure() { send({ cmd: "measure" }) }
  function checkLeaks() { send({ cmd: "checkLeaks" }) }
  function pasteBridges() { send({ cmd: "pasteBridges" }) }
  function applyBridges() { send({ cmd: "applyBridges" }) }
  function cancelBridges() { send({ cmd: "cancelBridges" }) }
  function setBridges(on) { send({ cmd: "setBridges", on: !!on }) }
  function refresh() { send({ cmd: "refresh" }) }

  onSettingsChanged: send({ cmd: "settings", settings: settings })

  Component.onCompleted: if (live) daemon.running = true
  onLiveChanged: {
    restart.stop()
    backoffMs = 2000
    daemon.running = live
  }

  property bool started: false
  // How long to wait before starting it again. Doubles while it keeps failing,
  // so a missing or broken helper costs one line in the shell's log a minute,
  // not one a second.
  property int backoffMs: 2000

  Process {
    id: daemon
    // The installed copy, by its fixed path: setup puts it there, and a plugin
    // update never changes what runs here until setup runs again.
    command: ["/usr/local/bin/tormarchyd"]
    // Started and stopped by hand below, never bound: Quickshell writes
    // `running` itself when the process exits, which would break a binding.
    running: false
    stdinEnabled: true

    stdout: SplitParser {
      onRead: function(line) {
        var data = null
        try { data = JSON.parse(line) } catch (e) { return }
        if (data && typeof data.view === "object" && data.view !== null) root.view = data.view
      }
    }

    onStarted: {
      root.started = true
      root.backoffMs = 2000
      root.send({ cmd: "hello", settings: root.settings, pluginDir: root.pluginDir })
      // The whole count, so a restart does not forget a panel still open.
      root.send({ cmd: "watchers", count: root.watchers })
    }

    // Quickshell reports a program it cannot find without an exit, only
    // running falling back to false; either way the next try is scheduled.
    onRunningChanged: {
      if (running) return
      root.started = false
      root.view = root.placeholder
      if (root.live) restart.restart()
    }
  }

  // Setup, started from the panel. pkexec raises Omarchy's own password dialog;
  // setup's lines become the progress the panel shows while it runs, and the
  // result arrives as a notification, since the panel closes for the dialog.
  property bool settingUp: false
  property string setupStatus: ""
  property string setupError: ""
  // No polkit agent to ask the password with: the panel falls back to a
  // terminal and sudo.
  signal setupNeedsTerminal()

  function runSetup() {
    if (setupRun.running) return
    setupError = ""
    setupStatus = "Waiting for your password..."
    settingUp = true
    setupRun.lastError = ""
    setupRun.noAgent = false
    setupRun.exitSeen = false
    setupRun.running = true
  }

  function notify(args) {
    if (settings && settings.notifications === "off") return
    notifier.command = ["omarchy-notification-send", "--app-name", "Tormarchy"].concat(args)
    notifier.running = true
  }

  Process {
    id: setupRun
    command: ["pkexec", root.pluginDir + "/setup"]
    running: false
    property string lastError: ""
    property bool noAgent: false
    property bool exitSeen: false

    stdout: SplitParser {
      onRead: function(line) {
        var t = line.trim()
        if (t !== "") root.setupStatus = t
      }
    }
    stderr: SplitParser {
      onRead: function(line) {
        var t = line.trim()
        if (t === "") return
        if (/authentication agent/i.test(t)) setupRun.noAgent = true
        setupRun.lastError = t.replace(/^(setup|tormarchy): /, "")
      }
    }

    onExited: function(code) {
      exitSeen = true
      root.settingUp = false
      root.setupStatus = ""
      if (code === 0) {
        root.notify(["-g", "󰒃", "Tor is set up", "Click the onion in the bar to connect.",
                     "--exec", "omarchy-shell", "tripleu.tor", "open"])
        // The helper was just installed or replaced: start the daemon again so
        // it is the new one, and soon, so the panel is not left saying "set up".
        restart.stop()
        root.backoffMs = 250
        if (daemon.running) daemon.running = false
        else daemon.running = true
      } else if (setupRun.noAgent) {
        root.setupNeedsTerminal()
      } else if (code === 126 || code === 127) {
        // The password dialog was dismissed, or the password refused.
      } else {
        root.setupError = setupRun.lastError || ("Setup failed (exit " + code + ")")
        root.notify(["-u", "critical", "Tor setup failed", root.setupError,
                     "--exec", "omarchy-shell", "tripleu.tor", "open"])
      }
    }

    // pkexec missing altogether: Quickshell drops running without an exit.
    // Checked once the exit, if there is one, has had its turn: which of the
    // two signals comes first is not promised.
    onRunningChanged: {
      if (running) return
      Qt.callLater(function() {
        if (setupRun.exitSeen || !root.settingUp) return
        root.settingUp = false
        root.setupStatus = ""
        root.setupNeedsTerminal()
      })
    }
  }

  Process { id: notifier; running: false }

  Timer {
    id: restart
    interval: root.backoffMs
    repeat: false
    onTriggered: {
      if (!root.live || daemon.running) return
      root.backoffMs = Math.min(60000, root.backoffMs * 2)
      daemon.running = true
    }
  }
}
