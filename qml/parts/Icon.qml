import QtQuick
import Snellar
import "../icons.js" as Icons

Item {
    id: icon

    property string name
    property color color: Theme.white
    property real size: Theme.iconMd
    property real stroke: 2
    property bool spinning: false

    NumberAnimation {
        target: icon
        property: "rotation"
        running: icon.spinning
        loops: Animation.Infinite
        from: 0
        to: 360
        duration: Theme.spin
        onRunningChanged: if (!running) icon.rotation = 0
    }

    implicitWidth: size
    implicitHeight: size

    Image {
        anchors.centerIn: parent
        width: icon.size
        height: icon.size
        opacity: icon.color.a
        sourceSize: Qt.size(Math.ceil(icon.size), Math.ceil(icon.size))
        source: icon.size > 0 ? Icons.url(icon.name, icon.color, icon.stroke) : ""
    }
}
