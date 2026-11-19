import QtQuick
import Snellar

Row {
    id: note
    required property Fetch fetch
    spacing: Theme.spaceSm

    Icon { anchors.verticalCenter: parent.verticalCenter; name: note.fetch.offline ? "offline" : "alert"; size: Theme.iconSm; color: Theme.text3 }
    Text {
        anchors.verticalCenter: parent.verticalCenter
        width: note.width - Theme.iconSm - againLink.implicitWidth - 2 * note.spacing
        text: Errors.text(note.fetch.failure)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        wrapMode: Text.WordWrap
    }
    Link {
        id: againLink
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Try again")
        color: Theme.accent
        font.pixelSize: Theme.fsCaption
        font.weight: Font.DemiBold
        onClicked: note.fetch.reload()
    }
}
