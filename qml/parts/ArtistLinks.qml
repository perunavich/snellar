pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: links
    property var artists: []
    property real fontSize: Theme.fsCallout
    property color color: Theme.text2
    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight
    clip: true

    Row {
        id: row
        Repeater {
            model: links.artists
            delegate: Row {
                required property var modelData
                required property int index
                Link {
                    text: parent.modelData.name
                    font.pixelSize: links.fontSize
                    color: links.color
                    onClicked: if (parent.modelData.id) Nav.artist(parent.modelData.id)
                }
                Text {
                    visible: parent.index < links.artists.length - 1
                    text: ", "
                    font.pixelSize: links.fontSize
                    color: links.color
                }
            }
        }
    }
}
