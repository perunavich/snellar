pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: btn
    property bool held: false
    property real radius: height / 2
    property color rest: "transparent"
    property color hover: Theme.washed(rest)
    property string tip
    default property alias label: layer.data
    readonly property Item face: layer
    readonly property bool hovered: area.containsMouse
    readonly property bool pressed: area.pressed
    readonly property real visualScale: lens.value
    readonly property color lit: Qt.tint(Theme.menu, Theme.fillGroup)
    readonly property color faceColor: Motion.blend(Motion.blend(rest, hover, warm.value), lit, glow.value)
    signal clicked()

    activeFocusOnTab: visible && enabled
    opacity: enabled ? 1 : Theme.disabledOpacity
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()

    onHeldChanged: {
        if (held || lens.squeezing || Motion.settleKick === 0 || Motion.reduced) return
        lens.zeta = Motion.settleDamping
        lens.response = Motion.settleResponse
        lens.kick(Motion.settleKick)
    }

    PressLens {
        id: lens
        down: area.pressed
        w: btn.width
        h: btn.height
    }

    SpringValue {
        id: glow
        reducible: true
        response: Motion.faceResponse
        target: btn.held || area.pressed ? 1 : 0
    }

    SpringValue {
        id: warm
        reducible: true
        response: Motion.faceResponse
        target: area.containsMouse ? 1 : 0
    }

    Rectangle {
        anchors.fill: parent
        radius: btn.radius
        scale: lens.value
        visible: !btn.held
        color: btn.faceColor
    }

    FocusRing { target: btn; radius: btn.radius; scale: lens.value }

    Item {
        id: layer
        anchors.fill: parent
        scale: lens.value
        opacity: btn.held ? 0 : 1
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        onClicked: btn.clicked()
    }

    TipLoader {
        want: area.containsMouse && !area.pressed && !btn.held && btn.tip.length > 0
        cut: area.pressed || btn.held
        hint: btn.tip
    }
}
