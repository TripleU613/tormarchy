import QtQuick
import Quickshell.Io
import qs.Commons
import "Model.js" as Model

Item {
  id: root

  property var settings: ({})

  // Truth from `tormarchy status --json`.
  property bool installed: false
  property bool torRunning: false
  property bool connected: false
  property int bootstrap: 0
  property string mode: ""
  property string exitIp: ""
  property string exitCountry: ""
  property string exitCountryName: ""
  property string requestedExit: ""
  property int circuitAgeSec: 0
  property var path: []
  property double bytesRead: 0
  property double bytesWritten: 0
  property string statusText: "Checking…"
  property var warnings: []

  // Optimistic connect state so the UI reacts the instant you click, rather
  // than waiting for a bootstrap that can take ten seconds. _desired is -1
  // while we just follow the real state, or 0/1 while a toggle catches up.
  property int _desired: -1
  readonly property bool active: _desired === -1 ? connected : (_desired === 1)
  readonly property bool bootstrapping: torRunning && !connected && bootstrap < 100
  property bool refreshing: false
  property string actionStatus: ""
  property string lastError: ""

  // Latency through the circuit. Measured on a timer, but only while the panel
  // is actually on screen -- see `watching`.
  //
  // Each measurement is a real HTTPS round trip over Tor, so this is not polled
  // like the status is: relay bandwidth is donated, and a reading nobody is
  // looking at is pure waste. Gating on the panel being open is what makes
  // continuous measurement reasonable rather than rude.
  //
  // It deliberately does NOT run while the panel is closed. A request to a
  // fixed host at a fixed interval, forever, is a traffic pattern -- a regular
  // beacon with a recognisable shape, which is the last thing a Tor tool should
  // add on the user's behalf without being asked.
  property bool watching: false
  // Seconds between readings on the live stream. Small numbers are affordable
  // now: each tick is one round trip on a connection that is already open, not
  // a fresh circuit handshake.
  readonly property int speedIntervalSec: intSetting("speedIntervalSec", 3, 1, 60)

  property int latencyMs: 0
  property bool measuringSpeed: false
  property string speedError: ""

  // The moment the circuit is no longer carrying traffic -- the toggle went off,
  // or the daemon stopped -- the last reading becomes a claim about a path that
  // no longer exists. Drop it so the gauge empties to "— ms" at once rather than
  // freezing on a stale number beside an OFF switch.
  onCircuitReadyChanged: {
    if (!circuitReady) {
      latencyMs = 0
      latencyHistory = []
      speedError = ""
      measuringSpeed = false
    }
  }

  // Every measurement taken this session, capped to what the sparkline shows.
  property var latencyHistory: []

  // Countries Tor currently has usable exit relays in. Loaded on demand, not
  // polled: it costs a walk of the whole consensus, and it barely changes.
  property var exitCountries: []
  property bool loadingExitCountries: false
  property bool exitCountriesLoaded: false

  readonly property int refreshIntervalSec: intSetting("refreshIntervalSec", 5, 2, 300)
  readonly property string defaultMode: Model.isValidMode(setting("mode", "lan")) ? String(setting("mode", "lan")) : "lan"
  // The mode to show selected: the live one when we have it, else the setting.
  readonly property string effectiveMode: mode !== "" ? mode : defaultMode

  // True only while a connect/disconnect/mode action is actually in flight.
  // Deliberately does NOT fold in the routine status poll: that fires every
  // second or two and each call makes real control-port round trips, so folding
  // it in here flashed the toggle's spinner and made it briefly refuse clicks on
  // every refresh -- an intermittent "the switch didn't take" that had nothing
  // to do with any action. `refreshing` already covers the poll for anything
  // that genuinely wants to know a status read is happening.
  readonly property bool busy: actionProcess.running

  // The latency card belongs to the "on" experience. It measures over the SOCKS
  // proxy, which answers whenever tor is up at all -- including the
  // disconnected-but-still-running state, and the gap between a disconnect click
  // and the daemon actually stopping. Gating the live stream on that raw
  // torRunning is what left a millisecond reading ticking under an OFF switch.
  // Gate it on the optimistic `active` state instead, so turning off stops the
  // gauge the instant the switch flips. Browser-only has no firewall "connected"
  // state but does have a live proxy, so it counts as on for as long as tor runs.
  readonly property bool circuitReady: torRunning && (active || effectiveMode === "socks")

  property string _statusOutput: ""
  property string _statusError: ""
  property string _actionOutput: ""
  property string _actionError: ""
  property string _exitListOutput: ""
  property string _speedOutput: ""

  function setting(name, fallback) {
    var value = settings ? settings[name] : undefined
    return value === undefined || value === null ? fallback : value
  }

  function intSetting(name, fallback, min, max) {
    var n = parseInt(String(setting(name, fallback)), 10)
    if (!isFinite(n)) n = fallback
    if (n < min) n = min
    if (n > max) n = max
    return n
  }

  function refresh() {
    if (statusProcess.running) return
    _statusOutput = ""
    _statusError = ""
    refreshing = true
    // Via bash so a missing helper is a real exit 127 we can report as
    // "not installed"; a bare argv would just fail to spawn instead.
    statusProcess.command = ["bash", "-c", "exec tormarchy status --json"]
    statusProcess.running = true
  }

  function applyStatus(raw) {
    var parsed = Model.parseStatus(raw)
    if (!parsed.ok) {
      lastError = parsed.lastError || "Failed to read Tor status"
      return
    }
    installed = parsed.installed
    torRunning = parsed.torRunning
    connected = parsed.connected
    bootstrap = parsed.bootstrap
    mode = parsed.mode
    exitIp = parsed.exitIp
    exitCountry = parsed.exitCountry
    exitCountryName = parsed.exitCountryName
    requestedExit = parsed.requestedExit
    circuitAgeSec = parsed.circuitAgeSec
    path = parsed.path
    bytesRead = parsed.bytesRead
    bytesWritten = parsed.bytesWritten
    warnings = parsed.warnings
    // Reality caught up to the pending connect/disconnect — stop overriding.
    if (_desired !== -1 && connected === (_desired === 1)) _desired = -1
    statusText = parsed.statusText !== "" ? parsed.statusText : Model.stateLabel(root)
    lastError = ""
  }

  // Every privileged helper escalates itself (sudo when it has a tty, pkexec
  // otherwise), exactly like omarchy-dns. From the shell there is no tty, so
  // this surfaces Omarchy's own polkit dialog and we never shell out to a
  // terminal just to ask for a password.
  function connect(requestedMode) {
    var target = Model.isValidMode(requestedMode) ? String(requestedMode) : defaultMode
    // SOCKS mode touches no firewall rules, so there is nothing to "connect".
    run(["tormarchy", "connect", target], target === "socks" ? -1 : 1, qsTr("Connecting…"))
  }

  function disconnect() {
    run(["tormarchy", "disconnect"], 0, qsTr("Disconnecting…"))
  }

  function toggleConnected() {
    if (active) disconnect()
    else connect(effectiveMode)
  }

  function setMode(requestedMode) {
    if (!Model.isValidMode(requestedMode) || String(requestedMode) === mode) return
    run(["tormarchy", "mode", String(requestedMode)], -1, qsTr("Switching to %1…").arg(Model.modeLabel(requestedMode)))
  }

  // Unprivileged: NEWNYM over the control port, authenticated with the cookie
  // our group membership lets us read. No restart, no password.
  function newCircuit() {
    // The measured latency belonged to the old path. Keeping it on screen next
    // to a new circuit would be quietly wrong, so drop it.
    latencyMs = 0
    speedError = ""
    latencyHistory = []
    run(["tormarchy", "newnym"], -1, qsTr("Requesting a new circuit…"))
    measureAfterChange.restart()
  }

  function measureSpeed() {
    // Same gate as the live stream: an on-demand reading while the toggle is off
    // would measure a proxy the user has already switched away from.
    if (speedProcess.running || !circuitReady) return
    measuringSpeed = true
    speedError = ""
    _speedOutput = ""
    speedProcess.command = ["bash", "-c", "exec tormarchy speed --json"]
    speedProcess.running = true
  }

  function loadExitCountries() {
    if (exitListProcess.running || !torRunning) return
    loadingExitCountries = true
    _exitListOutput = ""
    exitListProcess.command = ["bash", "-c", "exec tormarchy exit --list"]
    exitListProcess.running = true
  }

  function setExit(countryCode) {
    var cc = String(countryCode || "auto")
    run(["tormarchy", "exit", cc], -1, cc === "auto"
      ? qsTr("Clearing exit country…")
      : qsTr("Switching exit to %1…").arg(cc.toUpperCase()))
  }

  // The last resort: tear down every rule unconditionally, for when a wedged
  // ruleset has left the machine with no network to explain itself with.
  function panic() {
    run(["tormarchy", "panic"], 0, qsTr("Removing all Tor rules…"))
  }

  function run(argv, desired, progress) {
    if (actionProcess.running) return
    if (desired !== -1) _desired = desired
    _actionOutput = ""
    _actionError = ""
    actionStatus = String(progress || "")
    // Same reason as the status poll: go through bash so a missing helper
    // comes back as exit 127 instead of a process that never spawns.
    var quoted = []
    for (var i = 0; i < argv.length; i++) quoted.push(Util.shellQuote(String(argv[i])))
    actionProcess.command = ["bash", "-c", "exec " + quoted.join(" ")]
    actionProcess.running = true
  }

  Timer {
    id: refreshTimer
    interval: root.refreshIntervalSec * 1000
    repeat: true
    running: true
    triggeredOnStart: true
    onTriggered: root.refresh()
  }

  Timer {
    // While bootstrapping, the percentage is the whole story — poll fast
    // enough that it actually animates instead of jumping 0 -> 100.
    id: bootstrapPoll
    interval: 1000
    repeat: true
    running: root.bootstrapping || root._desired !== -1
    onTriggered: root.refresh()
  }

  Timer {
    id: delayedRefresh
    interval: 600
    repeat: false
    onTriggered: root.refresh()
  }

  // Live latency, streamed from one persistent connection through the circuit.
  //
  // A long-running child rather than a timer firing one-shot measurements: the
  // expensive part of a reading is building the connection, so reusing one and
  // re-timing it is what makes a live number affordable at all. Quickshell kills
  // the process when `running` goes false, so closing the panel stops it and
  // there is no daemon to leak.
  Process {
    id: pingProcess
    running: root.watching && root.circuitReady
    command: ["bash", "-c", "exec tormarchy pingd " + root.speedIntervalSec]

    stdout: SplitParser {
      onRead: function(line) {
        var ms = parseInt(String(line).trim(), 10)
        if (!isFinite(ms) || ms <= 0) return
        root.latencyMs = ms
        root.speedError = ""
        root.measuringSpeed = false
      }
    }

    stderr: SplitParser {
      onRead: function(line) {
        var text = String(line).trim()
        if (text !== "") root.speedError = Model.elide(text, 120)
      }
    }

    onRunningChanged: {
      // Show the spinner while the first reading is in flight; after that the
      // stream keeps the number current on its own.
      if (running && root.latencyMs <= 0) root.measuringSpeed = true
      else if (!running) root.measuringSpeed = false
    }
  }

  Timer {
    // A fresh circuit is a different path, so measure it rather than sitting
    // next to an empty gauge until the interval comes round. The delay lets
    // NEWNYM actually take effect first.
    id: measureAfterChange
    interval: 1500
    repeat: false
    onTriggered: if (root.watching) root.measureSpeed()
  }

  Timer {
    id: actionStatusTimer
    interval: 2600
    repeat: false
    onTriggered: root.actionStatus = ""
  }

  Timer {
    // Applying or flushing a ruleset settles quickly, but bootstrap does not.
    // Re-poll a handful of times so the panel reflects the new state without
    // waiting on the periodic refresh, then stop overriding reality.
    id: settleTimer
    property int ticks: 0
    interval: 1500
    repeat: true
    running: false
    onTriggered: {
      settleTimer.ticks += 1
      root.refresh()
      if (settleTimer.ticks >= 8) {
        settleTimer.ticks = 0
        settleTimer.running = false
        root._desired = -1
      }
    }
  }

  Process {
    id: statusProcess
    running: false
    command: []
    stdout: StdioCollector { id: statusStdout; waitForEnd: true; onStreamFinished: root._statusOutput = text }
    stderr: StdioCollector { id: statusStderr; waitForEnd: true; onStreamFinished: root._statusError = text }
    onExited: function(exitCode) {
      root.refreshing = false
      var stdout = String(statusStdout.text || root._statusOutput || "")
      var stderr = String(statusStderr.text || root._statusError || "")
      // A missing helper is the expected pre-setup state, not an error to
      // shout about: report "not installed" and let the panel offer setup.
      if (exitCode === 0) {
        root.applyStatus(stdout)
      } else if (exitCode === 127) {
        root.installed = false
        root.connected = false
        root.torRunning = false
        root.statusText = "Tor helpers are not installed"
        root.lastError = ""
      } else {
        root.lastError = Model.elide(stderr || stdout || "Could not read Tor status")
      }
    }
  }

  Process {
    id: actionProcess
    running: false
    command: []
    stdout: StdioCollector { id: actionStdout; waitForEnd: true; onStreamFinished: root._actionOutput = text }
    stderr: StdioCollector { id: actionStderr; waitForEnd: true; onStreamFinished: root._actionError = text }
    onExited: function(exitCode) {
      var stdout = String(actionStdout.text || root._actionOutput || "")
      var stderr = String(actionStderr.text || root._actionError || "")
      if (exitCode !== 0) {
        // Drop the optimistic state: whatever we hoped for did not happen.
        root._desired = -1
        root.lastError = Model.elide(stderr || stdout || "Tor command failed")
        root.actionStatus = root.lastError
        actionStatusTimer.restart()
      } else {
        root.lastError = ""
        actionStatusTimer.restart()
      }
      settleTimer.ticks = 0
      settleTimer.restart()
      delayedRefresh.restart()
    }
  }

  Process {
    id: exitListProcess
    running: false
    command: []
    stdout: StdioCollector { id: exitListStdout; waitForEnd: true; onStreamFinished: root._exitListOutput = text }
    onExited: function(exitCode) {
      root.loadingExitCountries = false
      if (exitCode !== 0) return
      var lines = String(exitListStdout.text || root._exitListOutput || "").split("\n")
      var codes = []
      for (var i = 0; i < lines.length; i++) {
        var cc = lines[i].trim().toLowerCase()
        if (cc.length === 2) codes.push(cc)
      }
      root.exitCountries = codes
      root.exitCountriesLoaded = true
    }
  }

  Process {
    id: speedProcess
    running: false
    command: []
    stdout: StdioCollector { id: speedStdout; waitForEnd: true; onStreamFinished: root._speedOutput = text }
    onExited: function(exitCode) {
      root.measuringSpeed = false
      var parsed = Model.parseSpeed(String(speedStdout.text || root._speedOutput || ""))
      if (parsed.ok) {
        root.latencyMs = parsed.latencyMs
        root.latencyHistory = Model.pushHistory(root.latencyHistory, parsed.latencyMs, 16)
        root.speedError = ""
      } else {
        root.latencyMs = 0
        root.speedError = parsed.error || qsTr("Could not measure the circuit")
      }
    }
  }
}
