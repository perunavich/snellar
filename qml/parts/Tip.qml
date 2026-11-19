import QtQuick
import QtQuick.Controls
import Snellar

ToolTip {
    id: tip
    property bool want: true
    property bool cut: false
    readonly property real level: fade.value
    readonly property bool fading: fade.value > 0 || fade.moving
    topPadding: Theme.spaceXs
    bottomPadding: Theme.spaceXs
    leftPadding: Theme.spaceSm
    rightPadding: Theme.spaceSm
    font.pixelSize: Theme.fsCaption
    enter: null
    exit: null
    opacity: Math.max(0, Math.min(1, fade.value))
    onCutChanged: if (cut) fade.snap(0)

    contentItem: Text {
        text: tip.text
        color: Theme.text
        font: tip.font
    }

    SpringValue {
        id: fade
        reducible: true
        zeta: Motion.tipDamping
        response: Motion.tipResponse
        target: tip.opened && tip.want ? 1 : 0
    }

    Item { id: nowhere }

    Binding {
        target: tip.contentItem.parent
        property: "containmentMask"
        value: nowhere
        when: !tip.want && tip.contentItem.parent !== null
    }

    Timer {
        running: tip.want
        interval: Theme.tipDelay
        onTriggered: tip.open()
    }

    background: Rectangle {
        radius: Theme.rSm
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator
    }
}
