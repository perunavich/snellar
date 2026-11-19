import QtQuick
import Snellar

Rectangle {
    property bool lineRight: true
    color: Theme.sidebar

    Rectangle {
        anchors { top: parent.top; bottom: parent.bottom }
        x: parent.lineRight ? parent.width - width : 0
        width: 1
        color: Theme.separator
    }
}
