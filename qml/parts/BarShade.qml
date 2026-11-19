import QtQuick
import Snellar

Rectangle {
    property bool edgeOnTop: false
    property bool edge: true
    color: Theme.pageBar

    Rectangle {
        anchors { left: parent.left; right: parent.right }
        y: parent.edgeOnTop ? 0 : parent.height - height
        height: 1
        visible: parent.edge
        color: Theme.hairline
    }
}
