import QtQuick
import Snellar

Item {
    id: glass
    property real radius: Theme.rPanel
    property color color: Theme.chrome
    property real edge: 1.0

    ShaderEffect {
        anchors.fill: parent
        property real wPx: width
        property real hPx: height
        property real radiusPx: glass.radius
        property vector4d tint: Qt.vector4d(glass.color.r, glass.color.g, glass.color.b, glass.color.a)
        property vector4d edgeTop: Qt.vector4d(Theme.edgeTop.r, Theme.edgeTop.g, Theme.edgeTop.b, Theme.edgeTop.a * glass.edge)
        property vector4d edgeBottom: Qt.vector4d(1, 1, 1, 0.07 * glass.edge)
        property vector4d hairline: Qt.vector4d(Theme.hairline.r, Theme.hairline.g, Theme.hairline.b, Theme.hairline.a * glass.edge)
        fragmentShader: "../../shaders/glass.frag.qsb"
    }
}
