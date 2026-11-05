import QtQuick
import QtQuick.Controls
import Snellar

ToolTip {
    id: tip
    topPadding: Theme.spaceXs
    bottomPadding: Theme.spaceXs
    leftPadding: Theme.spaceSm
    rightPadding: Theme.spaceSm
    font.pixelSize: Theme.fsCaption

    contentItem: Text {
        text: tip.text
        color: Theme.text
        font: tip.font
    }

    Timer {
        running: true
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
