import QtQuick
import Snellar

Pressable {
    id: btn
    property string text
    property string icon
    property bool on: false
    property real progress: -1
    property bool stalled: false

    width: Theme.controlLg
    height: Theme.controlLg
    rest: Theme.fillRaised
    tip: text

    Icon {
        anchors.centerIn: parent
        name: btn.icon
        size: Theme.iconMd
        color: btn.on ? Theme.accent : Theme.text
    }

    ProgressRing {
        anchors { fill: parent; margins: Theme.ring }
        visible: btn.progress >= 0
        value: btn.progress
        color: btn.stalled ? Theme.text3 : Theme.accent
    }
}
