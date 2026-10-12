import QtQuick
import QtQuick.Controls
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model

Panel {
  id: root
  moduleName: "tripleu.tor"
  ipcTarget: "tripleu.tor"
  manageIpc: false

  property string focusSection: "header"  // "setup" | "header" | "newnym" | "exit" | "speed" | "mode"

  // Follow the session's text direction. Without this an RTL locale keeps the
  // Latin left-to-right order, so the onion, the wordmark and every row read
  // backwards. childrenInherit pushes it down the whole tree, including the
  // rows built by Repeaters.
  LayoutMirroring.enabled: Qt.application.layoutDirection === Qt.RightToLeft
  LayoutMirroring.childrenInherit: true
  property int modeIndex: 0
  property bool cursorActive: false
  // Which tool tile is open beneath its band: "" | "leaks" | "bridges".
  property string expandedTool: ""

  // Every palette read goes through the Commons alias. Qt 6.12 ships a `Color`
  // type of its own, and a bare `Color.accent` now resolves to that instead of
  // the theme singleton -- it evaluates to undefined, and a colour property
  // given undefined falls back to black. That is what painted the switch's
  // "on" track black on a dark bar, so it looked as if it never turned on.
  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color urgent: bar ? bar.urgent : Commons.Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  // Straight from the theme singleton: the bar exposes foreground, barForeground
  // and urgent, but no accent, so there is nothing to prefer over it here.
  readonly property color accent: Commons.Color.accent
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // The onion's state, in order of what most needs saying.
  //
  // It only lights up once traffic is actually going through Tor. Bootstrapping
  // is a distinct state with a pulsing core -- claiming "on" before the circuit
  // exists is the one lie a Tor indicator must never tell. Browser-only with Tor
  // up keeps the core but not the layers: Tor is there, the system is not routed
  // through it. And rules applied with no Tor behind them is the kill switch
  // holding the network shut, which is worth the urgent colour: the machine has
  // gone quiet on purpose, and this is where the user will look to find out why.
  readonly property bool barBlocked: tor.connected && !tor.torRunning
  readonly property bool barRouted: tor.active && tor.connected && tor.torRunning
  readonly property bool barBrowserOnly: tor.active && tor.effectiveMode === "socks"
    && tor.torRunning && !tor.bootstrapping
  readonly property bool barConnecting: tor.active && !barRouted && !barBrowserOnly && !barBlocked

  readonly property color barIconColor: barBlocked ? urgent : barForeground
  readonly property real barLayerOpacity: barBlocked || barRouted ? 1.0 : 0.45

  // Our own directory, so the pre-setup path can point at the bundled
  // ./setup without the helpers being installed yet.
  readonly property string pluginDir: String(Qt.resolvedUrl("."))
    .replace(/^file:\/\//, "").replace(/\/$/, "")


  // In every mode, browser-only included. ExitNodes is a torrc setting, so it
  // steers every circuit tor builds -- the SOCKS circuits a browser-only
  // browser uses just as much as the transparent ones. Hiding it there left an
  // empty column in the header and took away a control that works.
  //
  // Deliberately NOT gated on torRunning. The exit tile is structural: hiding
  // it while disconnected collapses a column and the panel reads as broken
  // rather than as idle. It shows its placeholder and waits instead.
  readonly property bool showExit: tor.installed
  readonly property var recentExits: root.setting("recentExits", [])

  // The panel's only layout grid: three equal columns and two gutters.
  //
  // Every band derives its geometry from this, so column 3 begins at the same x
  // in all of them. What this replaces was two competing systems -- the header
  // and the latency row used a fixed rail width while the mode row took an
  // actual third of the available width, so the right-hand column sat ~25px off
  // and the panel read as crooked however the individual boxes were tweaked.
  //
  // Proportional rather than measured pixels: Style.space() is scaled per theme
  // (about 1.23 on this display), so a literal 11 would render as 13.5 here and
  // differently again elsewhere. Thirds are correct at every scale.
  readonly property real gridGap: Style.space(9)

  // Vertical rhythm, as multiples of that same gutter. Naming the two steps
  // stops the spacing being re-guessed per section, which is what made the
  // panel read as a pile of independently placed boxes.
  readonly property real gapTight: gridGap          // a label to its control
  readonly property real gapSection: gridGap * 2    // band to band, section to section

  // Fixed sizes from the layout spec. Expressed through Style.space so they
  // track the theme's spacing scale (about 1.23 here) and land on the intended
  // pixel values at that scale instead of being hardcoded to one display.
  readonly property real headerHeight: Style.space(52)   // 64px
  readonly property real controlHeight: Style.space(30)  // 37px
  readonly property real toggleWidth: Style.space(71)    // 88px
  readonly property real surfaceHeight: Style.space(78)  // 96px
  readonly property real gaugeCell: Style.space(91)      // 112px
  readonly property real gaugeSize: Style.space(68)      // 84px
  readonly property real metricsHeight: Style.space(39)  // 48px
  readonly property real modeHeight: Style.space(42)     // 52px
  readonly property real actionHeight: Style.space(29)   // 36px
  readonly property real toolHeight: Style.space(42)     // 52px, the mode row's height
  readonly property real hairline: Math.max(1, Style.space(1))

  readonly property color surfaceFill: Util.alpha(foreground, 0.05)
  readonly property color surfaceLine: Util.alpha(foreground, 0.22)

  // One definition for both status surfaces. Declaring the fill and border once
  // is what guarantees the latency card and New circuit are visually identical
  // -- two separately styled boxes drift the moment either is touched.
  component Surface: Rectangle {
    color: root.surfaceFill
    border.width: root.hairline
    border.color: root.surfaceLine
    radius: Style.cornerRadius
  }
  // A tool's tile in band 4: glyph, title, one line of state. Selecting it opens
  // the tool beneath the band.
  component ToolTile: Surface {
    id: tile
    property string section: ""
    property string glyph: ""
    property color glyphColor: root.foreground
    property string title: ""
    property string subtitle: ""
    property color subtitleColor: root.dim
    signal activated()

    readonly property bool hot: tileMouse.containsMouse
      || (root.cursorActive && root.focusSection === section)
    readonly property bool open: root.expandedTool === section

    height: root.toolHeight
    clip: true
    border.color: hot || open ? Util.alpha(root.foreground, 0.55) : root.surfaceLine

    Text {
      id: tileGlyph
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
        width: parent.width
        elide: Text.ElideRight
        text: tile.title
        color: root.foreground
        font.family: root.fontFamily
        font.pixelSize: Style.font.body
      }

      Text {
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
  }

  // Ticks only while the panel is open, so "checked 2 min ago" stays true.
  property double now: Date.now()
  Timer {
    interval: 30000
    repeat: true
    running: root.opened
    triggeredOnStart: true
    onTriggered: root.now = Date.now()
  }

  function gridColumn(available) { return (available - gridGap * 2) / 3 }
  function gridSpan2(available) { return gridColumn(available) * 2 + gridGap }

  // A labelled figure. Declared inline because it is only ever used by the
  // stats strip below and gains nothing from being its own file.
  component StatCell: Column {
    property string label: ""
    property string value: ""
    property color valueColor: root.foreground

    spacing: Style.space(2)
    // Nothing leaves the cell. Without this a long value simply draws over the
    // neighbouring cell, which is how "62 KB" and "15s" ended up printed on
    // top of each other.
    clip: true

    Text {
      width: parent.width
      elide: Text.ElideRight
      text: parent.label
      color: root.dim
      font.family: root.fontFamily
      font.pixelSize: Style.font.caption
      font.letterSpacing: 0.8
    }

    Text {
      width: parent.width
      elide: Text.ElideRight
      // A cell may supply its figure as child content instead of a string --
      // the circuit path does. An empty Text still claims a line of height in
      // a Column, which would push that content down out of alignment.
      visible: text !== ""
      text: parent.value
      color: parent.valueColor
      font.family: root.fontFamily
      font.pixelSize: Style.font.bodySmall
    }
  }


  // Reading order of the bands, skipping whatever is not on screen. Rebuilt
  // each call because tor stopping or socks mode removes sections under the
  // cursor.
  function navOrder() {
    var order = ["header"]
    if (showExit) order.push("exit")
    order.push("speed")
    if (tor.canNewCircuit) order.push("newnym")
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
    // A section can vanish under the cursor — tor stopping removes "exit".
    if (navOrder().indexOf(focusSection) === -1) focusSection = "header"
    modeIndex = Math.max(0, Model.MODE_ORDER.indexOf(tor.effectiveMode))
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

    if (dx === 0) return
    if (focusSection === "mode") {
      modeIndex = Math.max(0, Math.min(Model.MODE_ORDER.length - 1, modeIndex + dx))
    }
  }

  function activateCursor() {
    ensureCursor()
    if (focusSection === "setup") runSetup()
    else if (focusSection === "header") toggleConnected()
    else if (focusSection === "newnym") tor.newCircuit()
    else if (focusSection === "exit") exitPicker.toggle()
    else if (focusSection === "speed") tor.measureSpeed()
    else if (focusSection === "mode") tor.setMode(Model.MODE_ORDER[modeIndex])
    else if (focusSection === "leaks") openLeakCheck()
    else if (focusSection === "bridges") toggleTool("bridges")
  }

  function toggleTool(name) {
    expandedTool = expandedTool === name ? "" : name
    // A paste is only ever previewed while its section is on screen.
    if (expandedTool !== "bridges") tor.cancelBridges()
    else tor.loadBridgeDetail()
  }

  // Opening the leak check runs it, unless a result is already on screen: one
  // that is there describes the current state, because every routing change
  // clears it. Running it again is the explicit button beneath.
  function openLeakCheck() {
    if (expandedTool === "leaks") {
      expandedTool = ""
      return
    }
    expandedTool = "leaks"
    if (!tor.leakResult && !tor.checkingLeaks) tor.checkLeaks()
  }

  // canToggle rather than !busy: a connect still waiting on tor's bootstrap can
  // be switched off, which cancels it.
  function toggleConnected() {
    if (tor.installed && tor.canToggle) tor.toggleConnected()
  }

  function chooseExit(code) {
    var cc = String(code || "auto")
    tor.setExit(cc)
    if (cc !== "auto") persistRecentExit(cc)
  }

  // Same persistence route tailscale uses for its recent Mullvad regions:
  // rewrite this widget's own entry in shell.json, keeping every other setting.
  function persistRecentExit(code) {
    var cc = String(code || "")
    if (cc === "") return
    var next = [cc]
    for (var i = 0; i < recentExits.length && next.length < 4; i++) {
      var existing = String(recentExits[i] || "")
      if (existing !== "" && existing !== cc && next.indexOf(existing) === -1) next.push(existing)
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
    tor.loadExitCountries()
    exitPicker.open()
  }


  // Setup is the one genuinely interactive step (pacman, a polkit policy, a
  // group change), so it gets a real terminal instead of a silent pkexec.
  function runSetup() {
    if (!bar) return
    close()
    // A real terminal, not pkexec: setup refuses to run through polkit, since
    // the single action pins the program path and not its arguments, so a
    // passwordless grant for connect would otherwise cover uninstall too.
    bar.run("omarchy-launch-floating-terminal-with-presentation "
      + Util.shellQuote("sudo " + pluginDir + "/tormarchy setup"))
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // Measuring runs only while the panel is on screen. Closing it stops the
  // timer, so the plugin never makes a request nobody asked for and nobody
  // would see.
  onOpenedChanged: {
    tor.watching = opened
    if (!opened) {
      expandedTool = ""
      return
    }
    cursorActive = false
    if (panelFlick) panelFlick.contentY = 0
    tor.refresh()
    // Cheap to skip, expensive to fetch: only walk the consensus once per
    // shell session, and only when an exit could actually be chosen.
    if (showExit && !tor.exitCountriesLoaded) tor.loadExitCountries()
    ensureCursor()
    Qt.callLater(function() { keyCatcher.forceActiveFocus() })
  }

  Service {
    id: tor
    settings: root.settings
  }

  Connections {
    target: tor
    function onInstalledChanged() { root.ensureCursor() }
    function onConnectedChanged() { root.ensureCursor() }
    function onCanNewCircuitChanged() { root.ensureCursor() }
    function onModeChanged() { root.ensureCursor() }
  }

  // ShellIpc rather than a bare IpcHandler: it also registers with the shell's
  // own socket, so `omarchy-shell tripleu.tor toggle` reaches it without
  // starting a qs ipc client, the same as every first-party panel.
  ShellIpc {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): string { tor.refresh(); return "ok" }
    function connect(): string { tor.connect(tor.effectiveMode); return "ok" }
    function disconnect(): string { tor.disconnect(); return "ok" }
    function newCircuit(): string {
      if (!tor.canNewCircuit) return "tor is not connected"
      tor.newCircuit()
      return "ok"
    }
    function toggleTor(): string { root.toggleConnected(); return "ok" }
    function status(): string { return tor.statusText }
    function checkLeaks(): string { root.open(); root.openLeakCheck(); return "ok" }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // The mark alone, no label: the state is in the icon itself, and the
    // panel is one click away for anything more specific. The tooltip says it
    // in words for anyone who has not learned the icon yet.
    tooltipText: tor.statusText
    // No position of its own. BarIconContent measures what the mark paints,
    // scales it to the bar's icon size and centres it on the slot, exactly as
    // it does every first-party glyph -- placing it here as well only fought
    // that, and lost.
    iconComponent: Component {
      TorIcon {
        iconSize: Style.bar.iconSize
        color: root.barIconColor
        layerOpacity: root.barLayerOpacity
        coreVisible: root.barRouted || root.barBrowserOnly || root.barBlocked || root.barConnecting
        pulsing: root.barConnecting
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) root.toggleConnected()
      else if (buttonCode === Qt.MiddleButton) { if (tor.canNewCircuit) tor.newCircuit() }
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
        if (key === "t") root.toggleConnected()
        else if (key === "n") { if (tor.canNewCircuit) tor.newCircuit() }
        else if (key === "e") root.openExitPicker()
        else if (key === "s") tor.measureSpeed()
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

          // Connected layout, in three bands: identity and controls, the
          // circuit's measured latency, then the mode selector.
          Column {
            id: connectedBody
            visible: tor.installed
            width: parent.width
            spacing: root.gapSection

            readonly property real gap: root.gridGap
            readonly property real bandHeight: root.headerHeight

            // Band 1 -- logo and name, then the switch and the exit location
            // side by side on the right. All three are full band height, so the
            // row reads as three tiles rather than a stack.
            Row {
              id: topRow
              width: parent.width
              spacing: connectedBody.gap

              readonly property real switchWidth: root.toggleWidth
              readonly property real countryWidth: root.gridColumn(width)
              // The shell's own control height, so these sit the same as every
              // other control in the bar's panels rather than as slabs.
              readonly property real controlHeight: root.controlHeight
              // Brand and switch together fill columns 1 and 2 exactly, so the
              // switch's right edge lands on the column 2 boundary and the exit
              // tile starts precisely at column 3.
              readonly property real brandWidth: root.gridSpan2(width) - switchWidth - spacing

              Item {
                width: topRow.brandWidth
                height: connectedBody.bandHeight
                clip: true

                // The mark is capped short of the column so there is a clear
                // gap before the toggle, rather than the wordmark growing until
                // it touches it.
                Row {
                  anchors.verticalCenter: parent.verticalCenter
                  spacing: Style.space(8)

                  readonly property real markCap: Math.min(topRow.brandWidth - Style.space(16),
                                                           Style.space(193))

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

              // The switch tile. Its caption does double duty: the live
              // bootstrap percentage while connecting, the plain state
              // otherwise. That percentage is the one number worth having
              // during the slow part, and it had nowhere to appear before.
              Column {
                width: topRow.switchWidth
                spacing: Style.space(3)

                Text {
                  width: parent.width
                  horizontalAlignment: Text.AlignHCenter
                  text: tor.bootstrapping ? tor.bootstrap + "%"
                    : (tor.active ? qsTr("ON") : qsTr("OFF"))
                  color: tor.bootstrapping ? root.foreground : root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.letterSpacing: 0.8
                }

                // Just the track. The bordered tile that used to sit around it
                // was a box drawn around another box; the switch already reads
                // as a control on its own.
                Item {
                  width: parent.width
                  height: topRow.controlHeight

                  ToggleSwitch {
                    anchors.centerIn: parent
                    checked: tor.active
                    busy: tor.busy
                    hasCursor: root.cursorActive && root.focusSection === "header"
                    foreground: root.foreground
                    accent: root.accent
                    onToggled: root.toggleConnected()
                  }
                }
              }

              // The dropdown's own trigger is the tile -- no wrapper button
              // whose click would need forwarding into it.
              Column {
                width: topRow.countryWidth
                spacing: Style.space(3)
                visible: root.showExit

                // "EXIT" while idle; once a circuit exists, the country Tor
                // actually chose. The control below is the *request* -- on
                // Automatic it says nothing about where you came out, and this
                // is where that gets answered.
                Text {
                  width: parent.width
                  elide: Text.ElideRight
                  text: tor.circuitReady && tor.exitCountry !== ""
                    ? qsTr("EXIT · %1").arg(tor.exitCountry.toUpperCase())
                    : qsTr("EXIT")
                  horizontalAlignment: Text.AlignLeft
                  color: root.dim
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.caption
                  font.letterSpacing: 0.8
                }

                SearchableDropdown {
                  id: exitPicker
                  width: topRow.countryWidth
                  rowHeight: topRow.controlHeight
                  showLabel: false
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  // Short on purpose. The popup is only as wide as this trigger,
                  // and the shared component draws these strings without a width
                  // or elide -- a long one prints straight out of the popup and
                  // across the panel behind it.
                  placeholderText: qsTr("Search…")
                  emptyText: tor.loadingExitCountries ? qsTr("Loading…")
                    : (tor.torRunning ? qsTr("None found") : qsTr("Not connected"))
                  options: Model.exitOptions(tor.exitCountries)
                  value: tor.requestedExit === "" ? "auto" : tor.requestedExit
                  triggerLabel: Model.exitLabel(tor.requestedExit)
                  hasCursor: root.cursorActive && root.focusSection === "exit"
                  onChanged: function(value) { root.chooseExit(value) }
                  onHovered: function(isHovered) {
                    if (isHovered) { root.cursorActive = true; root.focusSection = "exit" }
                  }
                }
              }
            }

            // What the status reports as wrong, said where it will be read.
            // The one that matters most is the kill switch holding: rules on,
            // tor gone, the network shut on purpose. The switch still reads ON
            // then, correctly, and without this nothing on screen explained why
            // the machine had gone quiet.
            Text {
              width: parent.width
              visible: text !== ""
              wrapMode: Text.WordWrap
              textFormat: Text.PlainText
              text: tor.warnings.length > 0 ? String(tor.warnings[0]) : ""
              color: root.urgent
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }

            // Band 2 -- two paired surfaces of identical height and border:
            // the latency card, and New circuit beside it. They are built from
            // the same Surface component rather than styled separately, which
            // is the only way to be sure they stay identical.
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

                // The gauge gets a fixed cell with a hairline on its right; the
                // value takes everything left over. Together they fill the card,
                // so there is no dead middle region.
                Item {
                  id: gaugeCell
                  width: root.gaugeCell
                  height: parent.height

                  SpeedGauge {
                    anchors.centerIn: parent
                    size: root.gaugeSize
                    value: Model.latencyFraction(tor.latencyMs)
                    busy: tor.measuringSpeed
                    color: root.foreground
                    trackColor: Util.alpha(root.foreground, 0.18)
                  }

                  // Separator, not a border: only the inner edge is drawn.
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

                    // The reading appears here and only here. It used to be
                    // printed inside the gauge as well, which said the same
                    // thing twice in one card.
                    Text {
                      width: parent.width
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      text: Model.formatLatency(tor.latencyMs)
                      color: root.foreground
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.displayLarge
                    }

                    Text {
                      width: parent.width
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      visible: text !== ""
                      text: tor.speedError !== "" ? tor.speedError
                        : (tor.measuringSpeed ? qsTr("Measuring…") : Model.latencyVerdict(tor.latencyMs))
                      color: tor.speedError !== "" ? root.urgent : root.dim
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.caption
                    }
                  }
                }

                MouseArea {
                  anchors.fill: parent
                  cursorShape: Qt.PointingHandCursor
                  onClicked: tor.measureSpeed()
                }
              }

              // Same Surface, so the geometry and border cannot diverge from
              // the card. Disabled reduces opacity only -- the surface keeps
              // its exact size either way.
              Surface {
                id: newCircuit
                width: speedRow.circuitWidth
                height: root.surfaceHeight
                clip: true

                readonly property bool available: tor.canNewCircuit
                readonly property bool hot: circuitHover.containsMouse
                  || (root.cursorActive && root.focusSection === "newnym")

                border.color: hot && available ? Util.alpha(root.foreground, 0.55) : root.surfaceLine
                opacity: available ? 1.0 : 0.45

                Text {
                  anchors.centerIn: parent
                  width: parent.width - Style.space(12)
                  horizontalAlignment: Text.AlignHCenter
                  wrapMode: Text.WordWrap
                  text: qsTr("New circuit")
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

            // The stats strip. Everything here comes from Tor's own control
            // port, which is why it can show the whole path and not just the
            // exit: nothing else on the bar has access to this.
            Row {
              id: statsRow
              width: parent.width
              height: root.metricsHeight
              spacing: connectedBody.gap
              // circuitReady, not connected: browser-only has a real circuit with a
              // real path and was hiding all three figures because it installs no
              // rules, and in the routed modes this drops the strip the moment the
              // switch goes off instead of leaving a circuit on screen under it.
              visible: tor.circuitReady && tor.path.length > 0
              clip: true

              // Thirds that add up, rather than three hand-picked widths that
              // did not. Every cell elides inside its share.
              readonly property real cellWidth: root.gridColumn(width)

              StatCell {
                width: statsRow.cellWidth
                label: qsTr("CIRCUIT")
                value: ""

                // The signature Tor view: guard, middle, exit. Rendered as hops
                // with arrows rather than a plain string so the exit, the only
                // one the outside world sees, can carry full weight.
                Row {
                  width: parent.width
                  spacing: Style.space(3)
                  clip: true

                  Repeater {
                    model: Model.pathHops(tor.path)

                    Row {
                      required property var modelData
                      required property int index

                      spacing: Style.space(3)

                      Text {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: index > 0
                        text: "\u2192"
                        color: Util.alpha(root.foreground, 0.45)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                      }

                      Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: modelData
                        color: root.foreground
                        opacity: index === tor.path.length - 1 ? 1.0 : 0.6
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.bodySmall
                      }
                    }
                  }
                }
              }

              StatCell {
                width: statsRow.cellWidth
                label: qsTr("TRAFFIC")
                // Single space between the two figures: the wide gap was what
                // pushed this past its cell in the first place.
                value: "\u2193 " + Model.formatBytes(tor.bytesRead)
                            + " \u2191 " + Model.formatBytes(tor.bytesWritten)
              }

              StatCell {
                width: statsRow.cellWidth
                // "AGE" not "CIRCUIT AGE" -- the column sits under a heading
                // that already says circuit, and the longer label was the
                // widest thing in the row.
                label: qsTr("AGE")
                value: Model.formatAge(tor.circuitAgeSec)
              }
            }

            // Band 4 -- the tools: the leak check across two columns, bridges
            // in the third, on the same grid and Surface as the latency card so
            // the band lines up with everything above it. Each opens beneath
            // the band rather than in a popup, so it scrolls with the panel.
            Row {
              id: toolsRow
              width: parent.width
              spacing: connectedBody.gap

              ToolTile {
                width: root.gridSpan2(toolsRow.width)
                section: "leaks"
                glyph: tor.checkingLeaks ? "󰔟"
                  : (tor.leakResult ? Model.doctorGlyph(Model.doctorOverall(tor.leakResult)) : "󰕥")
                glyphColor: Model.doctorOverall(tor.leakResult) === "fail" ? root.urgent : root.foreground
                title: qsTr("Leak check")
                subtitle: tor.checkingLeaks ? qsTr("Checking…")
                  : (tor.leakError !== "" ? tor.leakError
                  : (tor.leakResult ? Model.doctorSummary(tor.leakResult) + " · " + Model.formatAgo((root.now - tor.leakCheckedAtMs) / 1000)
                  : qsTr("Prove nothing gets past Tor")))
                subtitleColor: tor.leakError !== "" ? root.urgent : root.dim
                onActivated: root.openLeakCheck()
              }

              ToolTile {
                width: root.gridColumn(toolsRow.width)
                section: "bridges"
                glyph: "󰘘"
                title: qsTr("Bridges")
                subtitle: Model.bridgeStateLabel(tor.bridgeState, tor.bridgeDetail)
                onActivated: root.toggleTool("bridges")
              }
            }

            // The leak check's results, one line per check, grouped the way
            // `tormarchy doctor` prints them in a terminal.
            Column {
              id: leakDetail
              visible: root.expandedTool === "leaks"
              width: parent.width
              spacing: Style.space(4)

              Repeater {
                model: tor.leakResult ? tor.leakResult.checks : []

                Column {
                  required property var modelData
                  required property int index

                  width: leakDetail.width
                  spacing: Style.space(2)

                  readonly property bool newGroup: index === 0
                    || tor.leakResult.checks[index - 1].group !== modelData.group

                  Text {
                    visible: parent.newGroup
                    topPadding: parent.index === 0 ? 0 : Style.space(4)
                    text: String(parent.modelData.group).toUpperCase()
                    color: root.dim
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.caption
                    font.letterSpacing: 0.8
                  }

                  Row {
                    width: parent.width
                    spacing: Style.space(8)

                    Text {
                      id: checkGlyph
                      width: Style.space(14)
                      text: Model.doctorGlyph(modelData.status)
                      color: modelData.status === "fail" ? root.urgent
                        : (modelData.status === "skip" ? root.dim : root.foreground)
                      font.family: root.fontFamily
                      font.pixelSize: Style.font.body
                    }

                    Column {
                      width: parent.width - checkGlyph.width - parent.spacing

                      Text {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        textFormat: Text.PlainText
                        text: modelData.text
                        color: modelData.status === "fail" ? root.urgent
                          : (modelData.status === "skip" ? root.dim : root.foreground)
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.bodySmall
                      }

                      Text {
                        visible: text !== ""
                        width: parent.width
                        wrapMode: Text.WordWrap
                        textFormat: Text.PlainText
                        text: modelData.detail
                        color: root.dim
                        font.family: root.fontFamily
                        font.pixelSize: Style.font.caption
                      }
                    }
                  }
                }
              }

              Text {
                visible: tor.checkingLeaks
                width: parent.width
                wrapMode: Text.WordWrap
                text: qsTr("Asking check.torproject.org and trying IPv6 directly. Up to half a minute.")
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }

              Button {
                visible: !tor.checkingLeaks
                text: tor.leakResult ? qsTr("Check again") : qsTr("Check now")
                bordered: true
                focusable: false
                foreground: root.foreground
                fontFamily: root.fontFamily
                onClicked: tor.checkLeaks()
              }
            }

            // Bridges: what is set up, whether this machine can run it, and a
            // way to change it that never puts a bridge line on a command line.
            // A paste is read straight off the clipboard, shown as counts for a
            // yes or no, and handed to tormarchy over stdin.
            Column {
              id: bridgeSection
              visible: root.expandedTool === "bridges"
              width: parent.width
              spacing: Style.space(6)

              readonly property var current: tor.bridgeDetail
              readonly property var preview: tor.bridgePreview
              readonly property var missing: Model.missingTransports(preview)
              readonly property bool canApply: !!preview && missing.length === 0
                && preview.invalidLines === 0 && !tor.busy && !tor.bridgeWorking

              Text {
                visible: !bridgeSection.preview
                width: parent.width
                wrapMode: Text.WordWrap
                text: tor.bridgeState === "active" ? qsTr("Tor reaches the network through these bridges.")
                  : (tor.bridgeState === "stashed" ? qsTr("Saved bridges are switched off. Tor connects directly.")
                  : qsTr("No bridges. Tor connects to the network directly, which is fine unless it is blocked where you are."))
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }

              // The saved set, one transport per line. A transport this machine
              // cannot run is the usual reason bridges "do nothing", so it says
              // what to install right where it is listed.
              Repeater {
                model: !bridgeSection.preview && bridgeSection.current && bridgeSection.current.ok
                  ? bridgeSection.current.transports : []

                Row {
                  required property var modelData
                  width: bridgeSection.width
                  spacing: Style.space(8)

                  Text {
                    id: transportGlyph
                    width: Style.space(14)
                    text: Model.doctorGlyph(modelData.available ? "pass" : "fail")
                    color: modelData.available ? root.foreground : root.urgent
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.body
                  }

                  Text {
                    width: parent.width - transportGlyph.width - parent.spacing
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    text: modelData.available
                      ? qsTr("%1 %2").arg(modelData.count).arg(modelData.name)
                      : qsTr("%1 %2 · not installed: %3").arg(modelData.count).arg(modelData.name).arg(modelData.installHint)
                    color: modelData.available ? root.foreground : root.urgent
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.bodySmall
                  }
                }
              }

              // What the paste holds, before anything is changed.
              Column {
                visible: !!bridgeSection.preview
                width: parent.width
                spacing: Style.space(4)

                Text {
                  width: parent.width
                  wrapMode: Text.WordWrap
                  text: qsTr("From the clipboard: %1").arg(Model.bridgeCountText(bridgeSection.preview))
                  color: root.foreground
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.body
                }

                Repeater {
                  model: bridgeSection.missing

                  Text {
                    required property var modelData
                    width: bridgeSection.width
                    wrapMode: Text.WordWrap
                    textFormat: Text.PlainText
                    text: qsTr("%1 · not installed: %2").arg(modelData.name).arg(modelData.installHint)
                    color: root.urgent
                    font.family: root.fontFamily
                    font.pixelSize: Style.font.bodySmall
                  }
                }

                Text {
                  visible: !!bridgeSection.preview && bridgeSection.preview.invalidLines > 0
                  width: parent.width
                  wrapMode: Text.WordWrap
                  text: bridgeSection.preview && bridgeSection.preview.invalidLines === 1
                    ? qsTr("1 line is not a bridge line. Copy only the lines that start with a transport, then paste again.")
                    : qsTr("%1 lines are not bridge lines. Copy only the lines that start with a transport, then paste again.")
                        .arg(bridgeSection.preview ? bridgeSection.preview.invalidLines : 0)
                  color: root.urgent
                  font.family: root.fontFamily
                  font.pixelSize: Style.font.bodySmall
                }

                Row {
                  spacing: Style.space(8)

                  Button {
                    text: qsTr("Use these")
                    bordered: true
                    focusable: false
                    enabled: bridgeSection.canApply
                    opacity: enabled ? 1 : 0.4
                    foreground: root.foreground
                    fontFamily: root.fontFamily
                    onClicked: tor.applyBridges()
                  }

                  Button {
                    text: qsTr("Cancel")
                    focusable: false
                    foreground: root.foreground
                    fontFamily: root.fontFamily
                    onClicked: tor.cancelBridges()
                  }
                }
              }

              Text {
                visible: tor.bridgeMessage !== ""
                width: parent.width
                wrapMode: Text.WordWrap
                textFormat: Text.PlainText
                text: tor.bridgeMessage
                color: root.urgent
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
              }

              Row {
                visible: !bridgeSection.preview
                spacing: Style.space(8)

                Button {
                  text: tor.bridgeWorking ? qsTr("Reading…") : qsTr("Paste bridges")
                  bordered: true
                  focusable: false
                  enabled: !tor.bridgeWorking && !tor.busy
                  opacity: enabled ? 1 : 0.4
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.pasteBridges()
                }

                Button {
                  visible: tor.bridgeState !== "off"
                  text: tor.bridgeState === "active" ? qsTr("Turn off") : qsTr("Turn on")
                  focusable: false
                  enabled: !tor.busy && !tor.bridgeWorking
                  opacity: enabled ? 1 : 0.4
                  foreground: root.foreground
                  fontFamily: root.fontFamily
                  onClicked: tor.setBridgesEnabled(tor.bridgeState !== "active")
                }
              }

              Text {
                visible: !bridgeSection.preview
                width: parent.width
                wrapMode: Text.WordWrap
                text: qsTr("Get bridges from bridges.torproject.org or @GetBridgesBot on Telegram, copy them, then paste.")
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }
          }

          // Pre-setup. The helpers are the whole feature, so this state gets a
          // hero rather than a warning: mark and wordmark, the reason we are
          // asking, then the single action. Surfaces follow Style.cornerRadius
          // like every shared control, so the cards match the rest of the bar
          // instead of importing their own shape language.
          Column {
            id: setupHero
            visible: !tor.installed
            width: parent.width
            spacing: Style.space(14)

            // Wordmark only -- the mark lives in the bar, and repeating it
            // here just competed with the name for the same glance. With the
            // onion gone the word takes the full panel width, so it derives a
            // larger block size and reads as the heading it is.
            TorWordmark {
              anchors.horizontalCenter: parent.horizontalCenter
              color: root.foreground
              maxWidth: setupHero.width
            }

            Column {
              width: parent.width
              spacing: Style.space(4)

              Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("One-time setup, then it's a toggle.")
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
              }

              Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("Installs tor and the helper in a terminal with sudo, so the switch never asks for a password.")
                color: root.dim
                font.family: root.fontFamily
                font.pixelSize: Style.font.caption
              }
            }

            Button {
              width: parent.width
              text: qsTr("Run setup")
              bordered: true
              focusable: false
              hasCursor: root.cursorActive && root.focusSection === "setup"
              foreground: root.foreground
              fontFamily: root.fontFamily
              verticalPadding: Style.space(12)
              onClicked: root.runSetup()
              onHovered: function(isHovered) {
                if (isHovered) { root.cursorActive = true; root.focusSection = "setup" }
              }
            }
          }

          PanelSeparator {
            visible: tor.installed
            foreground: root.foreground
          }

          // Band 3 -- mode, named by how much each one protects and ordered
          // most-protective first. The wire values stay lan/strict/socks.
          Column {
            visible: tor.installed
            width: parent.width
            spacing: root.gapTight

            PanelSectionHeader {
              text: qsTr("MODE")
              foreground: root.foreground
              fontFamily: root.fontFamily
            }

            // One connected control: a single outer rectangle with two
            // internal separators. Three bordered boxes with gaps between them
            // read as three unrelated buttons rather than one choice of three.
            Rectangle {
              id: modeControl
              width: parent.width
              height: root.modeHeight
              radius: Style.cornerRadius
              color: "transparent"
              border.width: root.hairline
              border.color: root.surfaceLine
              clip: true

              readonly property real segmentWidth: (width - border.width * 2) / 3

              Row {
                x: modeControl.border.width
                y: modeControl.border.width
                height: modeControl.height - modeControl.border.width * 2

                Repeater {
                  model: Model.MODE_ORDER

                  Item {
                    required property var modelData
                    required property int index

                    width: modeControl.segmentWidth
                    height: parent.height
                    clip: true

                    readonly property bool selected: tor.effectiveMode === String(modelData)
                    readonly property bool hot: segmentHover.containsMouse
                      || (root.cursorActive && root.focusSection === "mode" && root.modeIndex === index)

                    Rectangle {
                      anchors.fill: parent
                      color: parent.selected ? Util.alpha(root.foreground, 0.14)
                        : (parent.hot ? Util.alpha(root.foreground, 0.06) : "transparent")
                      Behavior on color { ColorAnimation { duration: 140 } }
                    }

                    // Divider between segments, drawn on the leading edge of
                    // every segment but the first.
                    Rectangle {
                      visible: index > 0
                      anchors.left: parent.left
                      anchors.top: parent.top
                      anchors.bottom: parent.bottom
                      width: root.hairline
                      color: root.surfaceLine
                    }

                    Text {
                      anchors.centerIn: parent
                      width: parent.width - Style.space(8)
                      horizontalAlignment: Text.AlignHCenter
                      elide: Text.ElideRight
                      text: Model.modeLabel(modelData)
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
                      onClicked: tor.setMode(modelData)
                      onContainsMouseChanged: if (containsMouse) {
                        root.cursorActive = true
                        root.focusSection = "mode"
                        root.modeIndex = index
                      }
                    }

                    PanelToolTip {
                      visible: segmentHover.containsMouse
                      text: Model.modeTooltip(modelData)
                    }
                  }
                }
              }
            }

          }

          // No prose in the panel. What each mode does lives on the segments as
          // hover tooltips, and the UDP/IPv6 caveats live in the README.
          //
          // There is deliberately no "Remove all rules" button. Switching the
          // toggle off already runs tormarchy-disconnect, which removes the
          // ruleset -- so the button was a second control for what the switch
          // does, sitting right beside it and inviting the question of how the
          // two differ. tormarchy-panic still exists as a command, which is
          // where it belongs: its entire purpose is being reachable when the
          // network is down and this panel is not an option.
          Text {
            visible: tor.lastError !== ""
            width: parent.width
            wrapMode: Text.WordWrap
            text: tor.lastError
            color: root.urgent
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
        }
      }
    }
  }
}
