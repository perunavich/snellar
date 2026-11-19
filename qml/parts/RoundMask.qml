import QtQuick

ShaderEffect {
    property real radius
    property size picture: Qt.size(width, height)
    readonly property real stretch: picture.width > 0 && picture.height > 0 && width > 0 && height > 0 ? picture.width * height / (picture.height * width) : 1
    property real wPx: width
    property real hPx: height
    property real radiusPx: radius
    property rect crop: Qt.rect(stretch > 1 ? (1 - 1 / stretch) / 2 : 0, stretch < 1 ? (1 - stretch) / 2 : 0, Math.min(1, 1 / stretch), Math.min(1, stretch))
    fragmentShader: Qt.resolvedUrl("../../shaders/round.frag.qsb")
}
