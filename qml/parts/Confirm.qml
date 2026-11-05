import QtQuick
import QtQuick.Controls
import Snellar

Popup {
    id: confirm
    property string title
    property string text
    property string action
    signal accepted()

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Theme.confirmWidth
    modal: true
    focus: true
    padding: Theme.spaceXl

    Overlay.modal: Rectangle { color: Theme.scrim }

    Shortcut {
        sequence: "Escape"
        enabled: confirm.opened
        onActivated: confirm.close()
    }

    background: Rectangle {
        radius: Theme.rLg
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator
    }

    contentItem: Column {
        spacing: Theme.spaceLg

        Text {
            width: parent.width
            text: confirm.title
            color: Theme.text
            font.pixelSize: Theme.fsHeadline
            font.weight: Font.DemiBold
            wrapMode: Text.WordWrap
        }
        Text {
            width: parent.width
            visible: text.length > 0
            text: confirm.text
            color: Theme.text2
            font.pixelSize: Theme.fsBody
            wrapMode: Text.WordWrap
        }
        Row {
            anchors.right: parent.right
            spacing: Theme.spaceSm

            PillButton {
                text: qsTr("Cancel")
                onClicked: confirm.close()
            }
            PillButton {
                text: confirm.action
                danger: true
                onClicked: {
                    confirm.close()
                    confirm.accepted()
                }
            }
        }
    }
}
