import QtQuick
import QtQuick.Shapes
import qs.Commons

// Circular latency gauge. An arc from 135° round to 45° — the open-bottom dial
// shape, so the gap reads as the bottom of a scale rather than a broken ring.
//
// Drawn with Shapes for the same reason as everything else here: it takes the
// theme colour directly and stays crisp at any size.
Item {
  id: root

  // 0..1. Anything outside is clamped rather than drawn off the end of the arc.
  property real value: 0
  property color color: Color.foreground
  property color trackColor: Util.alpha(Color.foreground, 0.18)
  property real thickness: Math.max(2, size * 0.11)
  property real size: Style.space(78)
  property bool busy: false

  // The dial leaves 90° open at the bottom: 135° start, 270° of travel.
  readonly property real startAngle: 135
  readonly property real sweep: 270
  readonly property real clamped: Math.max(0, Math.min(1, value))

  width: size
  height: size
  implicitWidth: size
  implicitHeight: size

  Shape {
    anchors.fill: parent
    antialiasing: true
    preferredRendererType: Shape.CurveRenderer

    // Track.
    ShapePath {
      strokeColor: root.trackColor
      strokeWidth: root.thickness
      fillColor: "transparent"
      capStyle: ShapePath.RoundCap

      PathAngleArc {
        centerX: root.size / 2
        centerY: root.size / 2
        radiusX: (root.size - root.thickness) / 2
        radiusY: (root.size - root.thickness) / 2
        startAngle: root.startAngle
        sweepAngle: root.sweep
      }
    }

    // Value.
    ShapePath {
      strokeColor: root.color
      strokeWidth: root.thickness
      fillColor: "transparent"
      capStyle: ShapePath.RoundCap

      PathAngleArc {
        centerX: root.size / 2
        centerY: root.size / 2
        radiusX: (root.size - root.thickness) / 2
        radiusY: (root.size - root.thickness) / 2
        startAngle: root.startAngle
        // A zero sweep still paints a round cap, which would show a stray dot
        // at the start of an empty gauge. Collapse the stroke instead.
        sweepAngle: root.sweep * root.clamped
        Behavior on sweepAngle { NumberAnimation { duration: 320; easing.type: Easing.OutCubic } }
      }
    }
  }

  // Indeterminate spin while a measurement is in flight.
  Shape {
    anchors.fill: parent
    visible: root.busy
    antialiasing: true
    preferredRendererType: Shape.CurveRenderer

    ShapePath {
      strokeColor: root.color
      strokeWidth: root.thickness
      fillColor: "transparent"
      capStyle: ShapePath.RoundCap

      PathAngleArc {
        id: spinArc
        centerX: root.size / 2
        centerY: root.size / 2
        radiusX: (root.size - root.thickness) / 2
        radiusY: (root.size - root.thickness) / 2
        startAngle: 0
        sweepAngle: 70
      }
    }
  }

  RotationAnimation {
    target: spinArc
    property: "startAngle"
    from: 0
    to: 360
    duration: 1100
    loops: Animation.Infinite
    running: root.busy
  }

  // Whatever the caller wants in the middle: a number, a dash, a label.
  default property alias content: centre.children

  Item {
    id: centre
    anchors.centerIn: parent
    width: root.size - root.thickness * 4
    height: width
  }
}
