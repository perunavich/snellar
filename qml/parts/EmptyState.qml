import QtQuick
import Snellar

Column {
    property string icon
    property string text
    spacing: Theme.spaceMd

    Icon { anchors.horizontalCenter: parent.horizontalCenter; name: parent.icon; size: Theme.iconXl; color: Theme.text3 }
    Text {
        anchors.horizontalCenter: parent.horizontalCenter
        text: parent.text
        color: Theme.text2
        font.pixelSize: Theme.fsBody
    }
}
