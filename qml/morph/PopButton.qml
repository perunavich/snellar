import QtQuick
import Snellar

Pressable {
    id: btn
    property string icon
    property real size: Theme.controlMd
    property real iconSize: Theme.iconMd
    property color color: Theme.text2
    property color activeColor: Theme.accent
    property bool active: false

    width: size
    height: size

    Icon {
        anchors.centerIn: parent
        name: btn.icon
        size: btn.iconSize
        color: btn.active ? btn.activeColor : btn.held ? Theme.text : btn.color
    }
}
