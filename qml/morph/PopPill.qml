import QtQuick
import Snellar

Pressable {
    id: pill
    property string text
    property string lead
    property string tail
    property color leadColor: Theme.text2
    property real pad: Theme.spaceMd
    property real gap: Theme.spaceXs
    property real textLimit: Infinity
    property alias font: words.font

    rest: Theme.fillRaised
    implicitWidth: Theme.snap(row.width + 2 * pad)
    implicitHeight: Theme.controlSm
    width: implicitWidth
    height: implicitHeight

    Row {
        id: row
        anchors.centerIn: parent
        spacing: pill.gap

        Icon {
            anchors.verticalCenter: parent.verticalCenter
            visible: pill.lead.length > 0
            name: pill.lead
            size: Theme.iconSm
            color: pill.leadColor
        }
        Text {
            id: words
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, pill.textLimit)
            text: pill.text
            elide: Text.ElideRight
            color: Theme.text
            font.pixelSize: Theme.fsCallout
        }
        Icon {
            anchors.verticalCenter: parent.verticalCenter
            visible: pill.tail.length > 0
            name: pill.tail
            size: Theme.iconSm
            color: Theme.text2
        }
    }
}
