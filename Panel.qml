import QtQuick
import QtQuick.Controls
import Quickshell.Io
import qs.Commons
import qs.Commons as Commons
import qs.Ui

// The bar widget and its panel. Drawing and input only: every label, verdict
// and decision comes from tormarchyd as the service's `view`, and every click
// goes back to it as a command.
Panel {
  id: root
  moduleName: "tripleu.tor"
  ipcTarget: "tripleu.tor"
  manageIpc: false

  property string focusSection: "header"  // "setup" | "header" | "exit" | "speed" | "newnym" | "leaks" | "bridges" | "mode"

  // Follow the session's text direction, down the whole tree.
  LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
  LayoutMirroring.childrenInherit: true
  property int modeIndex: 0
  property bool cursorActive: false
  // Which tool tile is open beneath its band: "" | "leaks" | "bridges".
  property string expandedTool: ""

  // Every palette read goes through the Commons alias: Qt 6.12 ships a `Color`
  // type of its own that a bare `Color.accent` resolves to instead.
  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color urgent: bar ? bar.urgent : Commons.Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property color accent: Commons.Color.accent
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  readonly property var v: tor.view

  // The onion's state comes from the daemon: lit only once traffic actually
  // goes through Tor, pulsing while connecting, red while the kill switch holds.
  readonly property bool barBlocked: v.bar === "blocked"
  readonly property bool barRouted: v.bar === "routed"
  readonly property bool barBrowserOnly: v.bar === "browser"
  readonly property bool barConnecting: v.bar === "connecting"
  readonly property color barIconColor: barBlocked ? urgent : barForeground
  readonly property real barLayerOpacity: barBlocked || barRouted ? 1.0 : 0.45

  // Our own directory, so the pre-setup path can reach the bundled setup.
  readonly property string pluginDir: String(Qt.resolvedUrl(".")).replace(/^file:\/\//, "").replace(/\/$/, "")

  // In every mode: ExitNodes steers browser-only's circuits as much as the
  // transparent ones. Structural, so not gated on tor running.
  readonly property bool showExit: tor.installed
  readonly property var recentExits: root.setting("recentExits", [])

  // The panel's only layout grid: three equal columns and two gutters.
  readonly property real gridGap: Style.space(9)
  readonly property real gapTight: gridGap
  readonly property real gapSection: gridGap * 2

  // Fixed sizes from the layout spec, through Style.space so they track the
  // theme's spacing scale.
  readonly property real headerHeight: Style.space(52)
  readonly property real controlHeight: Style.space(30)
  readonly property real toggleWidth: Style.space(71)
  readonly property real surfaceHeight: Style.space(78)
  readonly property real gaugeCell: Style.space(91)
  readonly property real gaugeSize: Style.space(68)
  readonly property real metricsHeight: Style.space(39)
  readonly property real modeHeight: Style.space(42)
  readonly property real toolHeight: Style.space(42)
  readonly property real hairline: Math.max(1, Style.space(1))

  readonly property color surfaceFill: Util.alpha(foreground, 0.05)
  readonly property color surfaceLine: Util.alpha(foreground, 0.22)

  // Tooltips the panel shows before the daemon can speak, wrapped by hand to
  // what a bubble holds: the shell's tooltip draws one line however long.
  readonly property string setupTip: "Once: asks for your password, then installs\ntor and the helper. After that the switch\nnever asks for a password."
  readonly property string updateTip: "This plugin was updated, but the helper it\nruns was not. Updating runs setup again,\nwhich asks for your password once."
  readonly property string leakTip: "Asks check.torproject.org whether you are\ncoming from Tor, tries an IPv6 host directly,\nand checks DNS, forwarding and the clock. Runs\nonly when you ask, and takes up to half a\nminute."
  readonly property string bridgesTip: "For networks that block Tor. Paste bridge\nlines from the clipboard to use them."
  readonly property string bridgesHelpTip: "Get bridges at bridges.torproject.org or from\n@GetBridgesBot on Telegram. Copy the lines,\nthen paste them here."
  readonly property string invalidTip: "Copy only the lines that start with a\ntransport, like obfs4, then paste again."

  // One definition for both status surfaces, so they stay identical.
  component Surface: Rectangle {
    color: root.surfaceFill
    border.width: root.hairline
    border.color: root.surfaceLine
    radius: Style.cornerRadius
  }

  // A tool's tile: glyph, title, one line of state. Selecting it opens the
  // tool beneath the band.
  component ToolTile: Surface {
    id: tile
    property string section: ""
    property string glyph: ""
    property color glyphColor: root.foreground
    property string title: ""
    property string subtitle: ""
    property color subtitleColor: root.dim
    property string tip: ""
    signal activated()

    readonly property bool hot: tileMouse.containsMouse
      || (root.cursorActive && root.focusSection === section)
    readonly property bool open: root.expandedTool === section

    height: root.toolHeight
    clip: true
    border.color: hot || open ? Util.alpha(root.foreground, 0.55) : root.surfaceLine

    Text {
      id: tileGlyph
      textFormat: Text.PlainText
      anchors.left: parent.left
      anchors.leftMargin: Style.space(12)
      anchors.verticalCenter: parent.verticalCenter
      text: tile.glyph
      color: tile.glyphColor
      font.family: root.fontFamily
      font.pixelSize: Style.font.title
    }

    Column {
      anchors.left: tileGlyph.right
      anchors.leftMargin: Style.space(10)
      anchors.right: parent.right
      anchors.rightMargin: Style.space(10)
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(1)

      Text {
        textFormat: Text.PlainText
        width: parent.width
        elide: Text.ElideRight
        text: tile.title
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
      }

      Text {
        textFormat: Text.PlainText
        width: parent.width
        elide: Text.ElideRight
        visible: text !== ""
        text: tile.subtitle
        color: tile.subtitleColor
        font.family: root.fontFamily
        font.pixelSize: Style.font.caption
      }
    }

    MouseArea {
      id: tileMouse
      anchors.fill: parent
      hoverEnabled: true
      cursorShape: Qt.PointingHandCursor
      onClicked: tile.activated()
      onContainsMouseChanged: if (containsMouse) {
        root.cursorActive = true
        root.focusSection = tile.section
      }
    }

    PanelToolTip {
      visible: tileMouse.containsMouse && tile.tip !== ""
      text: tile.tip
      fontFamily: root.fontFamily
    }
  }

  // No sentences on the panel: this dot carries one in its bubble.
  component InfoDot: Text {
    id: info
    property string tip: ""
    textFormat: Text.PlainText
    text: "\u{f02fd}"
    color: infoMouse.containsMouse ? root.foreground : root.dim
    font.family: root.fontFamily
    font.pixelSize: Style.font.body

    MouseArea {
      id: infoMouse
      anchors.fill: parent
      anchors.margins: -Style.space(4)
      hoverEnabled: true
      acceptedButtons: Qt.NoButton
    }

    PanelToolTip {
      visible: infoMouse.containsMouse && info.tip !== ""
      text: info.tip
      fontFamily: root.fontFamily
    }
  }

  // What setup is doing while it runs, or why it stopped.
  component SetupProgress: Column {
    width: parent ? parent.width : implicitWidth
    spacing: Style.space(4)
    Text {
      visible: tor.settingUp && tor.setupStatus !== ""
      width: parent.width
      elide: Text.ElideRight
      textFormat: Text.PlainText
      text: tor.setupStatus
      color: root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.caption
    }
    Brief { full: tor.settingUp ? "" : tor.setupError }
  }

  // A message as one short line, with the whole of it in the bubble. The
  // daemon sends both halves.
  component Brief: Text {
    id: brief
    property string full: ""
    property string short: ""
    property string tip: full
    visible: full !== ""
    width: parent ? parent.width : implicitWidth
    elide: Text.ElideRight
    maximumLineCount: 1
    textFormat: Text.PlainText
    text: short !== "" ? short : full
    color: root.urgent
    font.family: root.fontFamily
    font.pixelSize: Style.font.caption

    MouseArea {
      id: briefMouse
      anchors.fill: parent
      hoverEnabled: true
      acceptedButtons: Qt.NoButton
    }

    PanelToolTip {
      visible: briefMouse.containsMouse && brief.full !== brief.text
      text: brief.tip
      fontFamily: root.fontFamily
    }
  }

  // A result in a word, with a glyph for how it went and the detail on hover.
  component Chip: Surface {
    id: chip
    property string glyph: ""
    property string label: ""
    property string tip: ""
    property color tint: root.foreground
    width: chipRow.implicitWidth + Style.space(16)
    height: Style.space(26)
    border.color: tint === root.urgent ? root.urgent : root.surfaceLine

    Row {
      id: chipRow
      anchors.centerIn: parent
      spacing: Style.space(5)

      Text {
        textFormat: Text.PlainText
        anchors.verticalCenter: parent.verticalCenter
        text: chip.glyph
        color: chip.tint
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }

      Text {
        textFormat: Text.PlainText
        anchors.verticalCenter: parent.verticalCenter
        text: chip.label
        color: chip.tint
        font.family: root.fontFamily
        font.pixelSize: Style.font.bodySmall
      }
    }

    MouseArea {
      id: chipMouse
      anchors.fill: parent
      hoverEnabled: true
      acceptedButtons: Qt.NoButton
    }

    PanelToolTip {
      visible: chipMouse.containsMouse && chip.tip !== ""
      text: chip.tip
      fontFamily: root.fontFamily
    }
  }

  function gridColumn(available) { return (available - gridGap * 2) / 3 }
  function gridSpan2(available) { return gridColumn(available) * 2 + gridGap }

  // A labelled figure in the stats strip.
  component StatCell: Column {
    property string label: ""
    property string value: ""
    property color valueColor: root.foreground

    spacing: Style.space(2)
    clip: true

    Text {
      textFormat: Text.PlainText
      width: parent.width
      elide: Text.ElideRight
      text: parent.label
      color: root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.caption
      font.letterSpacing: 0.8
    }

    Text {
      textFormat: Text.PlainText
      width: parent.width
      elide: Text.ElideRight
      visible: text !== ""
      text: parent.value
      color: parent.valueColor
      font.family: root.fontFamily
      font.pixelSize: Style.font.bodySmall
    }
  }

  // Reading order of the bands, skipping whatever is not on screen.
  function navOrder() {
    var order = ["header"]
    if (showExit) order.push("exit")
    order.push("speed")
    if (v.canNewCircuit) order.push("newnym")
    order.push("leaks")
    order.push("bridges")
    order.push("mode")
    return order
  }

  function ensureCursor() {
    if (!tor.installed) {
      focusSection = "setup"
      return
    }
    if (focusSection === "setup") focusSection = "header"
    if (navOrder().indexOf(focusSection) === -1) focusSection = "header"
    for (var i = 0; i < v.modes.length; i++) if (v.modes[i].selected) modeIndex = i
  }

  function moveCursor(dx, dy) {
    cursorActive = true
    ensureCursor()
    if (focusSection === "setup") return
    if (dy !== 0) {
      var order = navOrder()
      var at = Math.max(0, order.indexOf(focusSection))
      focusSection = order[Math.max(0, Math.min(order.length - 1, at + dy))]
      return
    }
    if (dx !== 0 && focusSection === "mode")
      modeIndex = Math.max(0, Math.min(v.modes.length - 1, modeIndex + dx))
  }

  function activateCursor() {
    ensureCursor()
    if (focusSection === "setup" && !tor.settingUp) runSetup()
    else if (focusSection === "header") tor.toggle()
    else if (focusSection === "newnym") tor.newCircuit()
    else if (focusSection === "exit") exitPicker.toggle()
    else if (focusSection === "speed") tor.measure()
    else if (focusSection === "mode" && v.modes[modeIndex]) tor.setMode(v.modes[modeIndex].value)
    else if (focusSection === "leaks") openLeakCheck()
    else if (focusSection === "bridges") toggleTool("bridges")
  }

  function toggleTool(name) {
    expandedTool = expandedTool === name ? "" : name
    // A paste is only ever previewed while its section is on screen.
    if (expandedTool !== "bridges") tor.cancelBridges()
  }

  // Opening the leak check runs it, unless a result is already on screen: any
  // routing change clears it, so one that is there is current.
  function openLeakCheck() {
    if (expandedTool === "leaks") {
      expandedTool = ""
      return
    }
    expandedTool = "leaks"
    tor.cancelBridges()
    if (!v.leak.hasResult && !v.leak.checking) tor.checkLeaks()
  }

  function chooseExit(code) {
    var cc = String(code || "auto")
    tor.setExit(cc)
    if (cc !== "auto") persistRecentExit(cc)
  }

  // This widget's own entry in shell.json, keeping every other setting. The
  // daemon reads the list back through the settings it is sent.
  function persistRecentExit(code) {
    var next = [code]
    for (var i = 0; i < recentExits.length && next.length < 4; i++) {
      var existing = String(recentExits[i] || "")
      if (existing !== "" && existing !== code && next.indexOf(existing) === -1) next.push(existing)
    }
    if (!bar || !bar.shell || typeof bar.shell.updateEntryInline !== "function") return
    var entry = { id: moduleName }
    for (var key in settings) if (key !== "id") entry[key] = settings[key]
    entry.recentExits = next
    bar.shell.updateEntryInline(moduleName, entry)
  }

  function openExitPicker() {
    if (!showExit) return
    cursorActive = true
    focusSection = "exit"
    tor.loadExits()
    exitPicker.open()
  }

  // Setup asks for the password in Omarchy's own dialog and reports back in a
  // notification. The panel closes first so nothing of ours stands between the
  // dialog and the keyboard.
  function runSetup() {
    close()
    tor.runSetup()
  }

  // Where there is no polkit agent to show that dialog: a terminal and sudo.
  function runSetupInTerminal() {
    if (!bar) return
    close()
    bar.run("omarchy-launch-floating-terminal-with-presentation "
      + Util.shellQuote("sudo " + pluginDir + "/setup"))
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // Measuring and the detail reads run only while the panel is on screen.
  onOpenedChanged: {
    tor.watch(opened)
    if (!opened) {
      expandedTool = ""
      return
    }
    cursorActive = false
    if (panelFlick) panelFlick.contentY = 0
    ensureCursor()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  // The plugin's service, which the shell runs once for every monitor's widget.
  // Anything else -- an older shell, a replacement bar -- gets this widget's own
  // copy, idle whenever the shared one exists.
  readonly property var sharedTor: bar && bar.shell && typeof bar.shell.serviceFor === "function"
    ? bar.shell.serviceFor("tripleu.tor") : null
  readonly property var tor: sharedTor || ownTor

  Service {
    id: ownTor
    live: !root.sharedTor
  }

  Binding {
    target: root.tor
    property: "settings"
    value: root.settings
  }

  Component.onDestruction: if (opened) tor.watch(false)

  Connections {
    target: tor
    function onInstalledChanged() { root.ensureCursor() }
    function onSetupNeedsTerminal() { root.runSetupInTerminal() }
    function onViewChanged() { if (root.opened) root.ensureCursor() }
  }

  // Quickshell's own IpcHandler, which every Omarchy 4 shell reaches.
  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): string { tor.refresh(); return "ok" }
    function connect(): string { tor.connect(); return "ok" }
    function disconnect(): string { tor.disconnect(); return "ok" }
    function newCircuit(): string {
      if (!root.v.canNewCircuit) return "tor is not connected"
      tor.newCircuit()
      return "ok"
    }
    function toggleTor(): string { tor.toggle(); return "ok" }
    function status(): string { return root.v.statusText }
    // Asked for by name, so it always shows the check and runs it.
    function checkLeaks(): string {
      root.open()
      root.focusSection = "leaks"
      root.expandedTool = "leaks"
      tor.cancelBridges()
      tor.checkLeaks()
      return "ok"
    }
    function bridges(): string {
      root.open()
      root.focusSection = "bridges"
      if (root.expandedTool !== "bridges") root.toggleTool("bridges")
      return "ok"
    }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // The mark alone: the state is in the icon, and in words in the tooltip.
    tooltipText: root.v.statusText
    iconComponent: Component {
      TorIcon {
        iconSize: Style.bar.iconSize || Math.round((Style.bar.iconCanvas || 16) * 0.75)
        color: root.barIconColor
        layerOpacity: root.barLayerOpacity
        coreVisible: root.barRouted || root.barBrowserOnly || root.barBlocked || root.barConnecting
        pulsing: root.barConnecting
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) tor.toggle()
      else if (buttonCode === Qt.MiddleButton) { if (root.v.canNewCircuit) tor.newCircuit() }
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(440))
    contentHeight: panel.fittedContentHeight(column.implicitHeight, Style.space(520))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onMoveRequested: function(dx, dy) {
        if (!root.cursorActive) { root.cursorActive = true; return }
        root.moveCursor(dx, dy)
      }
      onActivateRequested: if (root.cursorActive) root.activateCursor()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(t) {
        var key = String(t || "").toLowerCase()
        if (key === "t") tor.toggle()
        else if (key === "n") { if (root.v.canNewCircuit) tor.newCircuit() }
        else if (key === "e") root.openExitPicker()
        else if (key === "s") tor.measure()
        else if (key === "r") tor.refresh()
        else if (key === "d") { root.cursorActive = true; root.focusSection = "leaks"; root.openLeakCheck() }
        else if (key === "b") { root.cursorActive = true; root.focusSection = "bridges"; root.toggleTool("bridges") }
      }

      Flickable {
        id: panelFlick
        anchors.fill: parent
        contentWidth: width
        contentHeight: column.implicitHeight
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.VerticalFlick
        interactive: contentHeight > height
        ScrollBar.vertical: ScrollBar { policy: ScrollBar.AsNeeded }

        Column {
          id: column
          width: panelFlick.width
          spacing: root.gapSection

          Column {
            id: connectedBody
            visible: tor.installed
            width: parent.width
            spacing: root.gapSection

            readonly property real gap: root.gridGap
            readonly property real bandHeight: root.headerHeight

            // Band 1: the mark and name, then the switch and the exit.
            Row {
              id: topRow
              width: parent.width
              spacing: connectedBody.gap

              readonly property real switchWidth: root.toggleWidth
              readonly property real countryWidth: root.gridColumn(width)
              readonly property real controlHeight: root.controlHeight
              readonly property real brandWidth: root.gridSpan2(width) - switchWidth - spacing

              Item {
                width: topRow.brandWidth
                height: connectedBody.bandHeight
                clip: true

                Row {
                  anchors.verticalCenter: parent.verticalCenter
                  spacing: Style.space(8)

                  readonly property real markCap: Math.min(topRow.brandWidth - Style.space(16), Style.space(193))

                  TorIcon {
                    anchors.verticalCenter: parent.verticalCenter
                    iconSize: Style.space(26)
                    color: root.foreground
                  }

                  TorWordmark {
                    anchors.verticalCenter: parent.verticalCenter
                    color: root.foreground
                    maxWidth: parent.markCap - Style.space(34)
                  }
                }
              }

              // The switch tile: its caption is the bootstrap percentage while
              // connecting, the plain state otherwise.
              Column {
                width: topRow.switchWidth
                spacing: Style.space(3)

                Text {
                  textFormat: Text.PlainText
                  width: parent.width
                  horizontalAlignment: Text.AlignHCenter
                  text: root.v.switchCaption
                  color: root.v.switchCaptionLit ? root.foreground : root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.letterSpacing: 0.8
                }

                Item {
                  width: parent.width
                  height: topRow.controlHeight

                  ToggleSwitch {
                    anchors.centerIn: parent
                    checked: root.v.active
                    busy: root.v.busy
                    hasCursor: root.cursorActive && root.focusSection === "header"
                    foreground: root.foreground
                    accent: root.accent
                    onToggled: tor.toggle()
                  }
                }
              }

              // The dropdown's own trigger is the tile. Dropdown, not
              // SearchableDropdown: that one carries an editable TextField, and
              // on Qt 6.10 and older every editable text field reads the whole
              // clipboard each time it changes.
              Column {
                width: topRow.countryWidth
                spacing: Style.space(3)
                visible: root.showExit

                Text {
                  textFormat: Text.PlainText
                  width: parent.width
                  elide: Text.ElideRight
                  text: root.v.exitCaption
                  horizontalAlignment: Text.AlignLeft
                  color: root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.letterSpacing: 0.8
                }

                Dropdown {
                  id: exitPicker
                  width: topRow.countryWidth
                  rowHeight: topRow.controlHeight
                  showLabel: false
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  options: root.v.exitOptions
                  value: root.v.exitValue
                  hasCursor: root.cursorActive && root.focusSection === "exit"
                  onChanged: function(value) { root.chooseExit(value) }
                  onHovered: function(isHovered) {
                    if (isHovered) { root.cursorActive = true; root.focusSection = "exit" }
                  }
                }
              }
            }

            // What the status reports as wrong, said where it will be read.
            Brief {
              full: root.v.warning.full
              short: root.v.warning.short
            }

            Button {
              visible: root.v.helperStale
              text: tor.settingUp ? "Updating helper..." : "Update helper"
              bordered: true
              focusable: false
              foreground: root.foreground
              fontFamily: root.fontFamily
              tooltipText: root.updateTip
              onClicked: if (!tor.settingUp) root.runSetup()
            }

            SetupProgress { visible: root.v.helperStale || tor.setupError !== "" }

            // Band 2: the latency card, and New circuit beside it.
            Row {
              id: speedRow
              width: parent.width
              spacing: connectedBody.gap

              readonly property real circuitWidth: root.gridColumn(width)
              readonly property real cardWidth: root.gridSpan2(width)

              Surface {
                id: latencyCard
                width: speedRow.cardWidth
                height: root.surfaceHeight
                clip: true

                Item {
                  id: gaugeCell
                  width: root.gaugeCell
                  height: parent.height

                  SpeedGauge {
                    anchors.centerIn: parent
                    size: root.gaugeSize
                    value: root.v.latency.fraction
                    busy: root.v.latency.measuring
                    color: root.foreground
                    trackColor: Util.alpha(root.foreground, 0.18)
                  }

                  Rectangle {
                    anchors.right: parent.right
                    anchors.top: parent.top
                    anchors.bottom: parent.bottom
                    width: root.hairline
                    color: root.surfaceLine
                  }
                }

                Item {
                  anchors.left: gaugeCell.right
                  anchors.right: parent.right
                  anchors.top: parent.top
                  anchors.bottom: parent.bottom
                  clip: true

                  Column {
                    anchors.centerIn: parent
                    width: parent.width - Style.space(16)
                    spacing: Style.space(3)

                    Text {
                      textFormat: Text.PlainText
                      width: parent.width
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      text: root.v.latency.text
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.displayLarge
                    }

                    Text {
                      textFormat: Text.PlainText
                      width: parent.width
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      visible: text !== ""
                      text: root.v.latency.caption
                      color: root.v.latency.captionError ? root.urgent : root.dim
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.caption
                    }
                  }
                }

                MouseArea {
                  anchors.fill: parent
                  cursorShape: Qt.PointingHandCursor
                  onClicked: tor.measure()
                }
              }

              Surface {
                id: newCircuit
                width: speedRow.circuitWidth
                height: root.surfaceHeight
                clip: true

                readonly property bool available: root.v.canNewCircuit
                readonly property bool hot: circuitHover.containsMouse
                  || (root.cursorActive && root.focusSection === "newnym")

                border.color: hot && available ? Util.alpha(root.foreground, 0.55) : root.surfaceLine
                opacity: available ? 1.0 : 0.45

                Text {
                  textFormat: Text.PlainText
                  anchors.centerIn: parent
                  width: parent.width - Style.space(12)
                  horizontalAlignment: Text.AlignHCenter
                  wrapMode: Text.WordWrap
                  text: "New circuit"
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                }

                MouseArea {
                  id: circuitHover
                  anchors.fill: parent
                  hoverEnabled: true
                  cursorShape: newCircuit.available ? Qt.PointingHandCursor : Qt.ArrowCursor
                  onClicked: if (newCircuit.available) tor.newCircuit()
                  onContainsMouseChanged: if (containsMouse) {
                    root.cursorActive = true
                    root.focusSection = "newnym"
                  }
                }
              }
            }

            // The stats strip, all from Tor's own control port.
            Row {
              id: statsRow
              width: parent.width
              height: root.metricsHeight
              spacing: connectedBody.gap
              visible: root.v.stats.visible
              clip: true

              readonly property real cellWidth: root.gridColumn(width)

              StatCell {
                width: statsRow.cellWidth
                label: "CIRCUIT"
                value: ""

                // Guard, middle, exit: the exit, the only one the outside world
                // sees, at full weight.
                Row {
                  width: parent.width
                  spacing: Style.space(3)
                  clip: true

                  Repeater {
                    model: root.v.stats.hops

                    Row {
                      required property var modelData
                      required property int index

                      spacing: Style.space(3)

                      Text {
                        textFormat: Text.PlainText
                        anchors.verticalCenter: parent.verticalCenter
                        visible: index > 0
                        text: "→"
                        color: Util.alpha(root.foreground, 0.45)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                      }

                      Text {
                        textFormat: Text.PlainText
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData.label
                        color: root.foreground
                        opacity: modelData.exit ? 1.0 : 0.6
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.bodySmall

                        MouseArea {
                          id: hopMouse
                          anchors.fill: parent
                          anchors.margins: -Style.space(2)
                          hoverEnabled: true
                          acceptedButtons: Qt.NoButton
                        }

                        PanelToolTip {
                          visible: hopMouse.containsMouse
                          text: modelData.tip
                          fontFamily: root.fontFamily
                        }
                      }
                    }
                  }
                }
              }

              StatCell {
                width: statsRow.cellWidth
                label: "TRAFFIC"
                value: root.v.stats.traffic
              }

              StatCell {
                width: statsRow.cellWidth
                label: "AGE"
                value: root.v.stats.age
              }
            }

            // Band 4: the tools.
            Row {
              id: toolsRow
              width: parent.width
              spacing: connectedBody.gap

              ToolTile {
                width: root.gridSpan2(toolsRow.width)
                section: "leaks"
                glyph: root.v.leak.glyph
                glyphColor: root.v.leak.failed ? root.urgent : root.foreground
                title: "Leak check"
                subtitle: root.v.leak.subtitle
                subtitleColor: root.v.leak.subtitleError ? root.urgent : root.dim
                tip: root.leakTip
                onActivated: root.openLeakCheck()
              }

              ToolTile {
                width: root.gridColumn(toolsRow.width)
                section: "bridges"
                glyph: "󰘘"
                title: "Bridges"
                subtitle: root.v.bridges.subtitle
                tip: root.bridgesTip
                onActivated: root.toggleTool("bridges")
              }
            }

            // The leak check's results: one chip per check.
            Column {
              id: leakDetail
              visible: root.expandedTool === "leaks"
              width: parent.width
              spacing: Style.space(8)

              Flow {
                visible: root.v.leak.hasResult
                width: parent.width
                spacing: Style.space(6)

                Repeater {
                  model: root.v.leak.checks

                  Chip {
                    required property var modelData
                    glyph: modelData.glyph
                    label: modelData.label
                    tip: modelData.tip
                    tint: modelData.status === "fail" ? root.urgent
                      : (modelData.status === "skip" ? root.dim : root.foreground)
                  }
                }
              }

              Button {
                visible: !root.v.leak.checking
                text: root.v.leak.hasResult ? "Check again" : "Check now"
                bordered: true
                focusable: false
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: tor.checkLeaks()
              }
            }

            // Bridges: what is set up and whether this machine can run it, then
            // a way to change it that never puts a bridge line on a command line.
            Column {
              id: bridgeSection
              visible: root.expandedTool === "bridges"
              width: parent.width
              spacing: Style.space(8)

              readonly property var b: root.v.bridges

              Text {
                textFormat: Text.PlainText
                visible: bridgeSection.b.hasPreview
                text: bridgeSection.b.previewText
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
              }

              Flow {
                visible: bridgeSection.b.chips.length > 0 || bridgeSection.b.invalidLabel !== ""
                width: parent.width
                spacing: Style.space(6)

                Repeater {
                  model: bridgeSection.b.chips

                  Chip {
                    required property var modelData
                    glyph: modelData.available ? "󰄬" : "󰅙"
                    label: modelData.label
                    tint: modelData.available ? root.foreground : root.urgent
                    tip: modelData.tip
                  }
                }

                Chip {
                  visible: bridgeSection.b.invalidLabel !== ""
                  glyph: "󰅙"
                  label: bridgeSection.b.invalidLabel
                  tint: root.urgent
                  tip: root.invalidTip
                }
              }

              Brief {
                full: bridgeSection.b.message.full
                short: bridgeSection.b.message.short
              }

              Row {
                spacing: Style.space(8)

                Button {
                  visible: bridgeSection.b.hasPreview
                  text: "Use these"
                  bordered: true
                  focusable: false
                  enabled: bridgeSection.b.canApply
                  opacity: enabled ? 1 : 0.4
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.applyBridges()
                }

                Button {
                  visible: bridgeSection.b.hasPreview
                  text: "Cancel"
                  focusable: false
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.cancelBridges()
                }

                Button {
                  visible: !bridgeSection.b.hasPreview
                  text: bridgeSection.b.working ? "Reading…" : "Paste bridges"
                  bordered: true
                  focusable: false
                  enabled: !bridgeSection.b.working && !root.v.busy
                  opacity: enabled ? 1 : 0.4
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.pasteBridges()
                }

                Button {
                  visible: !bridgeSection.b.hasPreview && bridgeSection.b.toggleLabel !== ""
                  text: bridgeSection.b.toggleLabel
                  focusable: false
                  enabled: !root.v.busy && !bridgeSection.b.working
                  opacity: enabled ? 1 : 0.4
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.setBridges(bridgeSection.b.state !== "active")
                }

                InfoDot {
                  anchors.verticalCenter: parent.verticalCenter
                  tip: root.bridgesHelpTip
                }
              }
            }
          }

          // Pre-setup: the wordmark, then the single action.
          Column {
            id: setupHero
            visible: !tor.installed
            width: parent.width
            spacing: Style.space(14)

            TorWordmark {
              anchors.horizontalCenter: parent.horizontalCenter
              color: root.foreground
              maxWidth: setupHero.width
            }

            Button {
              width: parent.width
              text: tor.settingUp ? "Setting up..." : "Set up"
              iconSpinning: tor.settingUp
              tooltipText: root.setupTip
              bordered: true
              focusable: false
              hasCursor: root.cursorActive && root.focusSection === "setup"
              foreground: root.foreground
              fontFamily: root.fontFamily
              verticalPadding: Style.space(12)
              onClicked: if (!tor.settingUp) root.runSetup()
              onHovered: function(isHovered) {
                if (isHovered) { root.cursorActive = true; root.focusSection = "setup" }
              }
            }

            SetupProgress {}
          }

          PanelSeparator {
            visible: tor.installed
            foreground: root.foreground
          }

          // Band 3: mode, most protective first.
          Column {
            visible: tor.installed
            width: parent.width
            spacing: root.gapTight

            PanelSectionHeader {
              text: "MODE"
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            Rectangle {
              id: modeControl
              width: parent.width
              height: root.modeHeight
              radius: Style.cornerRadius
              color: "transparent"
              border.width: root.hairline
              border.color: root.surfaceLine
              clip: true

              readonly property real segmentWidth: (width - border.width * 2) / Math.max(1, root.v.modes.length)

              Row {
                x: modeControl.border.width
                y: modeControl.border.width
                height: modeControl.height - modeControl.border.width * 2

                Repeater {
                  model: root.v.modes

                  Item {
                    required property var modelData
                    required property int index

                    width: modeControl.segmentWidth
                    height: parent.height
                    clip: true

                    readonly property bool selected: modelData.selected
                    readonly property bool hot: segmentHover.containsMouse
                      || (root.cursorActive && root.focusSection === "mode" && root.modeIndex === index)

                    Rectangle {
                      anchors.fill: parent
                      color: parent.selected ? Util.alpha(root.foreground, 0.14)
                        : (parent.hot ? Util.alpha(root.foreground, 0.06) : "transparent")
                      Behavior on color {
                        ColorAnimation {
                          duration: typeof Style.duration === "function" ? Style.duration(140) : 140
                        }
                      }
                    }

                    Rectangle {
                      visible: index > 0
                      anchors.left: parent.left
                      anchors.top: parent.top
                      anchors.bottom: parent.bottom
                      width: root.hairline
                      color: root.surfaceLine
                    }

                    Text {
                      textFormat: Text.PlainText
                      anchors.centerIn: parent
                      width: parent.width - Style.space(8)
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      text: modelData.label
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.body
                      font.bold: parent.selected
                    }

                    MouseArea {
                      id: segmentHover
                      anchors.fill: parent
                      hoverEnabled: true
                      cursorShape: Qt.PointingHandCursor
                      onClicked: tor.setMode(modelData.value)
                      onContainsMouseChanged: if (containsMouse) {
                        root.cursorActive = true
                        root.focusSection = "mode"
                        root.modeIndex = index
                      }
                    }

                    PanelToolTip {
                      visible: segmentHover.containsMouse
                      text: modelData.tip
                    }
                  }
                }
              }
            }
          }

          Brief {
            full: root.v.error.full
            short: root.v.error.short
          }
        }
      }
    }
  }
}
