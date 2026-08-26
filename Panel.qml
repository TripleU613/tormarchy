import QtQuick
import QtQuick.Controls
import Quickshell.Io
import qs.Commons
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

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  // Straight from the theme singleton: the bar exposes foreground, barForeground
  // and urgent, but no accent, so there is nothing to prefer over Color here.
  readonly property color accent: Color.accent
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // The onion only lights up once traffic is actually going through Tor.
  // Bootstrapping is a distinct, dimmer state — claiming "on" before the
  // circuit exists is the one lie a Tor indicator must never tell.
  readonly property color barIconColor: tor.active && tor.connected
    ? barForeground
    : Qt.darker(barForeground, tor.bootstrapping ? 1.25 : 1.55)

  // Our own directory, so the pre-setup path can point at the bundled
  // ./setup without the helpers being installed yet.
  readonly property string pluginDir: String(Qt.resolvedUrl("."))
    .replace(/^file:\/\//, "").replace(/\/$/, "")


  // Exit selection only makes sense once tor is up and actually carrying
  // traffic; in socks mode there is no system-wide circuit to steer.
  // Deliberately NOT gated on torRunning. The exit tile is structural: hiding
  // it while disconnected collapses a column and the panel reads as broken
  // rather than as idle. It shows its placeholder and waits instead.
  readonly property bool showExit: tor.installed && tor.effectiveMode !== "socks"
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
  }

  function toggleConnected() {
    if (tor.installed && !tor.busy) tor.toggleConnected()
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


  // Reuse the first-party speed test rather than rolling our own: while a
  // transparent ruleset is up, everything it measures already goes through
  // the circuit, so this reports Tor throughput for free.
  function summonSpeedTest() {
    if (!bar || !bar.shell) return
    close()
    var label = tor.connected
      ? ("Tor" + (tor.exitCountry ? " · " + tor.exitCountry.toUpperCase() : ""))
      : ""
    bar.shell.summon("omarchy.speedtest", label ? JSON.stringify({ connection: label }) : "{}")
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
    if (!opened) return
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

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): string { tor.refresh(); return "ok" }
    function connect(): string { tor.connect(tor.effectiveMode); return "ok" }
    function disconnect(): string { tor.disconnect(); return "ok" }
    function newCircuit(): string { tor.newCircuit(); return "ok" }
    function status(): string { return tor.statusText }
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // The mark alone. No label of any kind: state is carried by the icon's
    // colour, and the panel is one click away for anything more specific.
    iconComponent: Component {
      TorIcon {
        // Rounded rather than anchors.centerIn: the button's width and the
        // glyph's can differ in parity, and centring then lands the mark on a
        // half pixel even once the glyph itself is a whole number of them.
        x: Math.round((parent.width - width) / 2)
        y: Math.round((parent.height - height) / 2)
        iconSize: Style.space(13)
        color: root.barIconColor
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.RightButton) root.toggleConnected()
      else if (buttonCode === Qt.MiddleButton) tor.newCircuit()
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
        else if (key === "n") tor.newCircuit()
        else if (key === "e") root.openExitPicker()
        else if (key === "s") tor.measureSpeed()
        else if (t === "S") root.summonSpeedTest()
        else if (key === "r") tor.refresh()
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
                  text: tor.connected && tor.exitCountry !== ""
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
                text: qsTr("To continue, we need privileged rights.")
                color: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.body
              }

              Text {
                width: parent.width
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("Setup installs tor, the helpers, and a polkit rule.")
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
