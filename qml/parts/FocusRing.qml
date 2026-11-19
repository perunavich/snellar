import QtQuick
import Snellar

Rectangle {
    id: ring
    required property Item target
    property bool keyed: false
    objectName: "focusRing"
    anchors.fill: parent
    color: "transparent"
    border.width: Theme.ring
    border.color: Theme.text
    visible: keyed && target.activeFocus

    Connections {
        target: ring.target
        function onActiveFocusChanged() { ring.keyed = ring.target.activeFocus && Nav.byKeyboard() }
    }
    Connections {
        target: Nav
        function onPointingChanged() { if (Nav.pointing) ring.keyed = false }
    }
}
