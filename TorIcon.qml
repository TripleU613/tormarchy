import QtQuick
import QtQuick.Shapes
import qs.Commons

// The Tor mark, taken from the Tor Browser "Stable" icon and rendered as a
// single monochrome path.
//
// The source SVG is three stacked layers: a lavender disc, this path filled
// with a purple gradient, and a mirrored half-disc with a drop shadow. None of
// that survives here on purpose -- a bar glyph has to be one flat colour that
// follows the theme. So only path#center is kept, and it takes `color`.
//
// Drawn with Shapes rather than loaded as an Image: an Image would need a
// colour-overlay effect to be themeable at all, and would resample a 512px
// canvas down to ~13px. A path scales to whatever size is asked for.
//
// The geometry is even-odd, and that is load-bearing: the outer subpath is the
// full disc and the ring bands are cut out of it. Filling this non-zero paints
// a solid blob with no onion in it.
Item {
  id: root

  property real iconSize: Style.font.icon
  property color color: Color.foreground

  // The source artwork's canvas. Every coordinate below is in these units.
  readonly property real sourceSize: 512

  width: iconSize
  height: iconSize
  implicitWidth: iconSize
  implicitHeight: iconSize

  Item {
    width: root.sourceSize
    height: root.sourceSize
    transformOrigin: Item.TopLeft
    scale: root.iconSize / root.sourceSize

    Shape {
      anchors.fill: parent
      antialiasing: true
      preferredRendererType: Shape.CurveRenderer

      ShapePath {
        fillColor: root.color
        strokeWidth: 0
        strokeColor: "transparent"
        fillRule: ShapePath.OddEvenFill

        PathSvg { path: "M256.525143,465.439707 L256.525143,434.406609 C354.826191,434.122748 434.420802,354.364917 434.420802,255.992903 C434.420802,157.627987 354.826191,77.8701558 256.525143,77.5862948 L256.525143,46.5531962 C371.964296,46.8441537 465.446804,140.489882 465.446804,255.992903 C465.446804,371.503022 371.964296,465.155846 256.525143,465.439707 Z M256.525143,356.820314 C311.970283,356.529356 356.8487,311.516106 356.8487,255.992903 C356.8487,200.476798 311.970283,155.463547 256.525143,155.17259 L256.525143,124.146588 C329.115485,124.430449 387.881799,183.338693 387.881799,255.992903 C387.881799,328.654211 329.115485,387.562455 256.525143,387.846316 L256.525143,356.820314 Z M256.525143,201.718689 C286.266674,202.00255 310.3026,226.180407 310.3026,255.992903 C310.3026,285.812497 286.266674,309.990353 256.525143,310.274214 L256.525143,201.718689 Z M0,255.992903 C0,397.384044 114.60886,512 256,512 C397.384044,512 512,397.384044 512,255.992903 C512,114.60886 397.384044,0 256,0 C114.60886,0 0,114.60886 0,255.992903 Z" }
      }
    }
  }
}
