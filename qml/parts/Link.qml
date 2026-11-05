import QtQuick
import Snellar

Text {
    id: link
    signal clicked()
    color: Theme.text2
    font.pixelSize: Theme.fsCallout
    font.underline: area.containsMouse
    elide: Text.ElideRight

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: link.clicked()
    }
}
