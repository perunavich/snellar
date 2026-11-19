pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: links
    property var artists: []
    property real fontSize: Theme.fsCallout
    property color color: Theme.text2

    component Name: Row {
        property real natural
    }

    implicitWidth: {
        let w = 0
        for (let i = 0; i < names.count; i++) {
            const one = names.itemAt(i) as Name
            if (one) w += one.natural
        }
        return w
    }
    implicitHeight: row.implicitHeight
    clip: true

    Row {
        id: row
        Repeater {
            id: names
            model: links.artists
            delegate: Name {
                id: one
                required property var modelData
                required property int index
                readonly property bool last: index === links.artists.length - 1
                natural: link.implicitWidth + (last ? 0 : comma.implicitWidth)
                readonly property real room: links.width - x
                visible: room > 0
                Link {
                    id: link
                    width: one.natural <= one.room ? implicitWidth : Math.max(0, one.room)
                    text: one.modelData.name
                    font.pixelSize: links.fontSize
                    color: links.color
                    warms: one.modelData.id ? { method: "artist", id: one.modelData.id } : null
                    hit: 0
                    onClicked: if (one.modelData.id) Nav.artist(one.modelData.id)
                }
                Text {
                    id: comma
                    visible: !one.last && one.natural <= one.room
                    text: ", "
                    font.pixelSize: links.fontSize
                    color: links.color
                }
            }
        }
    }
}
