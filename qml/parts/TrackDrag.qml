import QtQuick
import Snellar

Item {
    id: proxy
    property var tracks: []
    readonly property bool dragging: Drag.active
    z: 100
    Drag.keys: ["tracks"]
    Drag.dragType: Drag.Internal

    function begin(list, at) {
        tracks = list
        place(at)
        Drag.active = true
    }

    function place(at) {
        const local = parent.mapFromItem(null, at.x, at.y)
        x = local.x
        y = local.y
    }

    function finish() {
        if (Drag.active) Drag.drop()
        Drag.active = false
        tracks = []
    }

    Rectangle {
        x: Theme.spaceMd
        y: Theme.spaceMd
        visible: proxy.dragging
        width: label.implicitWidth + 2 * Theme.spaceMd
        height: Theme.controlSm
        radius: height / 2
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator

        Text {
            id: label
            anchors.centerIn: parent
            text: proxy.tracks.length === 1 ? proxy.tracks[0].title : qsTr("%n song(s)", "", proxy.tracks.length)
            color: Theme.text
            font.pixelSize: Theme.fsCallout
            font.weight: Font.Medium
        }
    }
}
