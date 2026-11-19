import QtQuick
import Snellar

Column {
    id: block
    property string icon
    property string text
    property string detail
    property string actionText
    property string actionIcon
    signal acted()
    topPadding: Theme.spaceXl
    spacing: Theme.spaceMd

    Icon { anchors.horizontalCenter: parent.horizontalCenter; name: block.icon; size: Theme.iconXl; color: Theme.text3 }
    Column {
        anchors.horizontalCenter: parent.horizontalCenter
        width: Math.min(block.width, Theme.stateWidth)
        spacing: Theme.spaceXs

        Text {
            width: parent.width
            visible: text.length > 0
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: block.text
            color: Theme.text
            font.pixelSize: Theme.fsHeadline
            font.weight: Font.DemiBold
        }
        Text {
            width: parent.width
            visible: text.length > 0
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: block.detail
            color: Theme.text2
            font.pixelSize: Theme.fsBody
        }
    }
    PillButton {
        anchors.horizontalCenter: parent.horizontalCenter
        visible: block.actionText.length > 0
        text: block.actionText
        icon: block.actionIcon
        onClicked: block.acted()
    }
}
