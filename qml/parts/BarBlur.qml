import QtQuick
import QtQuick.Effects
import Snellar

Item {
    id: fx
    property var source
    property real amount: 0
    property real floor: 0
    property real floorAmount: 0
    property rect floorBox: Qt.rect(0, height - floor, width, floor)
    property real floorRadius: 0
    readonly property bool frosted: floor > 0 && floorAmount > 0

    component Frost: MultiEffect {
        autoPaddingEnabled: false
        blurEnabled: true
        blur: 1
        blurMax: Theme.barBlur
        saturation: Theme.barSaturation
    }

    Frost {
        id: floorFrost
        width: fx.width
        height: fx.height
        source: fx.source
        visible: false
        layer.enabled: fx.frosted
    }

    ShaderEffect {
        anchors.fill: parent
        property var source: fx.source
        property var blurred: fx.frosted ? floorFrost : fx.source
        property real cut: Theme.navBar / Math.max(1, height)
        property real amount: fx.amount
        property real floorAmount: fx.floor > 0 ? fx.floorAmount : 0
        property real floorRadius: fx.floorRadius
        property size size: Qt.size(width, height)
        property vector4d floorBox: Qt.vector4d(fx.floorBox.x, fx.floorBox.y, fx.floorBox.x + fx.floorBox.width, fx.floorBox.y + fx.floorBox.height)
        fragmentShader: "../../shaders/cut.frag.qsb"
    }

    Item {
        width: fx.width
        height: Theme.navBar
        clip: true
        visible: fx.amount > 0
        opacity: fx.amount

        Frost { width: fx.width; height: fx.height; source: fx.source }
    }
}
