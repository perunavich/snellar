import QtQuick
import Snellar

Text {
    id: link
    signal clicked()
    property var warms: null
    property int hit: Theme.hitMin
    color: Theme.text2
    font.pixelSize: Theme.fsCallout
    font.underline: area.containsMouse
    elide: Text.ElideRight

    MouseArea {
        id: area
        readonly property real reach: Math.max(0, (link.hit - link.height) / 2)
        anchors { fill: parent; topMargin: -reach; bottomMargin: -reach }
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: link.clicked()
    }

    Intent { target: link; hovered: area.containsMouse; page: link.warms }
}
