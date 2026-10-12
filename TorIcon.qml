import QtQuick
import qs.Commons
import qs.Commons as Commons

// The onion, as concentric layers: an outer skin, an inner layer, and a core.
//
// This replaced a monochrome copy of the Tor Browser artwork, which was the
// wrong shape for a bar. That artwork is a solid disc with ring bands cut out
// of its right half only -- its mirrored half is a separate, lighter layer in
// the original -- so flattened to one colour at 12px it rendered as a solid
// left half beside a smudge of rings: a moon, leaning left. The host centres a
// bar icon by its painted bounds, so no amount of nudging could fix an
// imbalance that lives inside the shape. A symmetric mark has none.
//
// Built from Rectangles, not a Shape path. BarIconContent measures what an icon
// paints to scale and centre it, and a Rectangle's bounds are exact and
// immediate, where a Shape's depend on the renderer having processed the path.
//
// The core carries the state, so it reads by shape as well as by colour --
// which also keeps it legible on light themes, where a dimmed glyph and a lit
// one can be closer in colour than a dark theme suggests:
//
//   filled core    traffic is going through Tor
//   pulsing core   Tor is bootstrapping
//   no core        off
Item {
  id: root

  property real iconSize: Style.font.icon
  property color color: Commons.Color.foreground

  // Opacity of the two outer layers. The bar dims them while the system as a
  // whole is not routed through Tor, including browser-only with Tor up.
  property real layerOpacity: 1.0
  property bool coreVisible: true
  property bool pulsing: false

  // Whole pixels: a fractional box gets centred onto a half pixel, which
  // antialiases into a soft mark.
  readonly property real size: Math.max(6, Math.round(iconSize))

  // Proportions chosen at the 12px bar size, where every layer and every gap
  // has to stay at least a pixel wide or the rings run together.
  readonly property real skin: Math.max(1, size * 0.12)
  readonly property real layerSize: size * 0.58
  readonly property real layerStroke: Math.max(1, size * 0.1)
  readonly property real coreSize: Math.max(2, size * 0.22)

  width: size
  height: size
  implicitWidth: size
  implicitHeight: size

  Rectangle {
    anchors.fill: parent
    radius: width / 2
    color: "transparent"
    border.width: root.skin
    border.color: root.color
    opacity: root.layerOpacity
    antialiasing: true
  }

  Rectangle {
    x: (root.size - width) / 2
    y: (root.size - height) / 2
    width: root.layerSize
    height: width
    radius: width / 2
    color: "transparent"
    border.width: root.layerStroke
    border.color: root.color
    opacity: root.layerOpacity
    antialiasing: true
  }

  Rectangle {
    id: core
    x: (root.size - width) / 2
    y: (root.size - height) / 2
    width: root.coreSize
    height: width
    radius: width / 2
    color: root.color
    antialiasing: true
    // Hidden by opacity rather than `visible`, so the painted bounds the bar
    // centres on stay the same in every state and the mark never shifts.
    opacity: !root.coreVisible ? 0 : (root.pulsing ? pulse.level : 1)

    QtObject {
      id: pulse
      property real level: 1
    }

    SequentialAnimation {
      running: root.pulsing && root.coreVisible
      loops: Animation.Infinite
      onRunningChanged: if (!running) pulse.level = 1
      NumberAnimation { target: pulse; property: "level"; from: 1; to: 0.25; duration: 650; easing.type: Easing.InOutSine }
      NumberAnimation { target: pulse; property: "level"; from: 0.25; to: 1; duration: 650; easing.type: Easing.InOutSine }
    }
  }
}
