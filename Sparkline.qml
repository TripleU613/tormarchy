import QtQuick
import qs.Commons

// Latency history as bars.
//
// Bars rather than a polyline: at forty pixels tall a line through a dozen
// points is mostly slope and no value, while bars stay readable and land on
// whole pixels — the same reason the wordmark is a pixel grid.
//
// Scaled to the largest value in the window, not to a fixed ceiling, so the
// shape shows how this circuit varies rather than where it sits against an
// arbitrary maximum. One flat bar means one sample, which is honest.
Item {
  id: root

  property var values: []
  property color color: Color.foreground
  property real barWidth: Math.max(2, Style.space(3))
  property real barGap: Math.max(1, Style.space(1))
  property int capacity: 16

  readonly property real peak: {
    var max = 0
    for (var i = 0; i < (values || []).length; i++) max = Math.max(max, Number(values[i]) || 0)
    return max
  }

  implicitWidth: capacity * barWidth + (capacity - 1) * barGap
  implicitHeight: Style.space(22)

  Row {
    anchors.bottom: parent.bottom
    anchors.left: parent.left
    spacing: root.barGap
    layoutDirection: Qt.LeftToRight

    Repeater {
      model: root.values

      Rectangle {
        required property var modelData
        required property int index

        // The most recent sample is the one being read, so it keeps full
        // strength while older ones fade back into the track.
        readonly property bool latest: index === root.values.length - 1
        readonly property real fraction: root.peak > 0 ? (Number(modelData) || 0) / root.peak : 0

        width: root.barWidth
        // A measured sample always draws something, so a fast reading next to a
        // slow one is a short bar rather than an absent one.
        height: Math.max(Math.max(1, Style.space(2)), root.height * fraction)
        radius: 0
        antialiasing: false
        color: root.color
        opacity: latest ? 1.0 : 0.30 + 0.35 * (index / Math.max(1, root.values.length - 1))
      }
    }
  }
}
