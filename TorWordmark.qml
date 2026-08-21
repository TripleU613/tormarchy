import QtQuick
import qs.Commons

// "TORMARCHY" as a 5x7 bitmap wordmark.
//
// Drawn from a pixel grid rather than set in a font, for the same reason
// TorIcon draws the onion instead of shipping an SVG: a plugin cannot assume
// any particular font is installed, and no pixel font ships with Omarchy or
// Arch by default. A grid renders identically on every machine, stays crisp at
// any size because the blocks are whole pixels, and takes the theme colour.
Item {
  id: root

  property color color: Color.foreground

  // Size of one block. Ignored when maxWidth is set.
  property real blockSize: 4

  // When > 0, the block size is derived so the word fills exactly this width.
  // Whole pixels only -- fractional blocks are what make pixel art look muddy.
  property real maxWidth: 0

  property int letterGap: 1

  readonly property string word: "TORMARCHY"
  readonly property int glyphWidth: 5
  readonly property int glyphHeight: 7

  readonly property var glyphs: ({
    "T": ["11111", "00100", "00100", "00100", "00100", "00100", "00100"],
    "O": ["01110", "10001", "10001", "10001", "10001", "10001", "01110"],
    "R": ["11110", "10001", "10001", "11110", "10100", "10010", "10001"],
    "M": ["10001", "11011", "10101", "10001", "10001", "10001", "10001"],
    "A": ["01110", "10001", "10001", "11111", "10001", "10001", "10001"],
    "C": ["01110", "10001", "10000", "10000", "10000", "10001", "01110"],
    "H": ["10001", "10001", "10001", "11111", "10001", "10001", "10001"],
    "Y": ["10001", "10001", "01010", "00100", "00100", "00100", "00100"]
  })

  readonly property int unitsWide: word.length * glyphWidth + (word.length - 1) * letterGap
  readonly property real block: maxWidth > 0
    ? Math.max(1, Math.floor(maxWidth / unitsWide))
    : blockSize

  readonly property var cells: root.buildCells()

  function buildCells() {
    var out = []
    var cursor = 0
    for (var i = 0; i < word.length; i++) {
      var rows = glyphs[word.charAt(i)]
      if (rows) {
        for (var y = 0; y < rows.length; y++) {
          var row = rows[y]
          for (var x = 0; x < row.length; x++) {
            if (row.charAt(x) === "1") out.push({ px: cursor + x, py: y })
          }
        }
      }
      cursor += glyphWidth + letterGap
    }
    return out
  }

  implicitWidth: unitsWide * block
  implicitHeight: glyphHeight * block
  width: implicitWidth
  height: implicitHeight

  Repeater {
    model: root.cells

    Rectangle {
      required property var modelData

      x: modelData.px * root.block
      y: modelData.py * root.block
      width: root.block
      height: root.block
      color: root.color
      // Blocks are whole pixels on a whole-pixel grid; smoothing them would
      // only blur the edges we are drawing on purpose.
      antialiasing: false
    }
  }
}
