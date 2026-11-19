import QtQuick
import Snellar

Rectangle {
    id: banner
    property bool active: false
    property string icon: "alert"
    property color tint: Theme.text2
    property string text
    property string actionText
    readonly property real room: parent ? parent.width : Theme.toastText
    signal acted()

    width: Theme.snap(row.width + 2 * Theme.pillPadding)
    height: Math.max(Theme.controlLg, Theme.snap(row.height + 2 * Theme.spaceSm))
    radius: Math.min(height / 2, Theme.rLg)
    color: Theme.toastFill
    border.width: 1
    border.color: Theme.hairline
    opacity: active ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { Ease { duration: Theme.m3 } }

    Row {
        id: row
        anchors.centerIn: parent
        spacing: Theme.spaceMd

        Icon { anchors.verticalCenter: parent.verticalCenter; name: banner.icon; size: Theme.iconSm; color: banner.tint }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, Theme.toastText, banner.room - 2 * Theme.pillPadding - Theme.iconSm - row.spacing - (action.visible ? action.implicitWidth + row.spacing : 0))
            text: banner.text
            color: Theme.text
            font.pixelSize: Theme.fsBody
            wrapMode: Text.WordWrap
        }
        Link {
            id: action
            anchors.verticalCenter: parent.verticalCenter
            visible: banner.actionText.length > 0
            text: banner.actionText
            color: Theme.accent
            font.pixelSize: Theme.fsBody
            font.weight: Font.DemiBold
            onClicked: banner.acted()
        }
    }
}
