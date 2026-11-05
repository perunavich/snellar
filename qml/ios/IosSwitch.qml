import QtQuick
import Snellar

Item {
    id: root
    property bool on: false
    signal toggled()

    width: Theme.switchWidth
    height: Theme.switchHeight
    opacity: enabled ? 1 : Theme.disabledOpacity

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: root.on ? Theme.accent : Theme.fillStrong
        Behavior on color { ColorAnimation { duration: Theme.normal } }

        Rectangle {
            width: parent.height - 2 * Theme.segmentInset
            height: width
            radius: width / 2
            color: root.on ? Theme.accentInk : Theme.white
            Behavior on color { ColorAnimation { duration: Theme.normal } }
            y: Theme.segmentInset
            x: root.on ? parent.width - width - Theme.segmentInset : Theme.segmentInset
            Behavior on x { NumberAnimation { duration: Theme.slide; easing.type: Easing.OutBack; easing.overshoot: 1.1 } }
        }
    }

    MouseArea {
        anchors.fill: parent
        enabled: root.enabled
        cursorShape: Qt.PointingHandCursor
        onClicked: root.toggled()
    }
}
