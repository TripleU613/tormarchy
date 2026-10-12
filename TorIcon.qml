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
  readonly property int size: Math.max(6, Math.round(iconSize))

  // Whole pixels throughout, each layer the same parity as the mark, so every
  // edge lands on the pixel grid and every layer centres exactly: at 12px that
  // is a 1px skin, a 1px gap, an 8px layer, another 1px gap and a 4px core. A
  // fractional size centred in a whole one sits half a pixel off and blurs.
  function evenWith(value, reference) {
    var n = Math.round(value)
    if (n % 2 === reference % 2) return n
    return value > n ? n + 1 : n - 1
  }
  readonly property int skin: Math.max(1, Math.round(size * 0.1))
  readonly property int layerSize: evenWith(size * 0.62, size)
  readonly property int layerStroke: Math.max(1, Math.round(size * 0.1))
  readonly property int coreSize: Math.max(2, evenWith(size * 0.3, size))

  width: size
  height: size
  implicitWidth: size
  implicitHeight: size

  // Every layer centres on the box this item is actually given, never on
  // `size`. Omarchy 4.0.x stretches a custom bar icon over its whole icon
  // canvas, wider than the mark; later shells keep the declared size and
  // centre what is painted. Layers placed against `size` drifted up and to the
  // left inside the stretched box, which is what put the mark off centre in
  // the bar. Centred on the box, it is right under both.
  Rectangle {
    x: (root.width - width) / 2
    y: (root.height - height) / 2
    width: root.size
    height: width
    radius: width / 2
    color: "transparent"
    border.width: root.skin
    border.color: root.color
    opacity: root.layerOpacity
    antialiasing: true
  }

  Rectangle {
    x: (root.width - width) / 2
    y: (root.height - height) / 2
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
    x: (root.width - width) / 2
    y: (root.height - height) / 2
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
