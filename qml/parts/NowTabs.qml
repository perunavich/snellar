import QtQuick
import Snellar

Item {
    id: tabs
    property int lineSize: Theme.fsTitle2

    IosSegment {
        id: bar
        anchors { left: parent.left; right: parent.right; top: parent.top }
        current: Nav.tab
        options: [{ key: "next", label: qsTr("Next") }, { key: "lyrics", label: qsTr("Lyrics") }, { key: "related", label: qsTr("Related") }]
        onPicked: key => Nav.tab = key
    }

    Item {
        id: head
        anchors { left: parent.left; right: parent.right; top: bar.bottom; topMargin: Theme.spaceSm }
        height: Theme.controlSm
        visible: Nav.tab === "next"
        readonly property int upcoming: Math.max(0, Api.queue.length - Api.cursor - 1)

        Text {
            anchors { left: parent.left; verticalCenter: parent.verticalCenter }
            text: head.upcoming > 0 ? qsTr("%n next", "", head.upcoming) : qsTr("Nothing next")
            color: Theme.text3
            font.pixelSize: Theme.fsCaption
        }
        Link {
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            visible: Api.queue.length > (Api.current ? 1 : 0)
            text: qsTr("Clear queue")
            font.pixelSize: Theme.fsCaption
            onClicked: Api.clearQueue()
        }
    }

    QueueList {
        id: queue
        anchors { left: parent.left; right: parent.right; top: head.bottom; bottom: parent.bottom }
        visible: Nav.tab === "next"
    }
    DropArea {
        id: queueDrop
        anchors.fill: queue
        enabled: queue.visible
        keys: ["tracks"]
        onDropped: {
            Api.enqueue(Nav.carried.tracks, false)
            Api.toast(qsTr("Added to queue"))
        }
        Rectangle {
            anchors.fill: parent
            radius: Theme.rMd
            color: Theme.accentFill
            border.width: 1
            border.color: Theme.accent
            visible: queueDrop.containsDrag
        }
    }
    LyricsView {
        anchors { left: parent.left; right: parent.right; top: bar.bottom; bottom: parent.bottom; topMargin: Theme.spaceMd }
        visible: Nav.tab === "lyrics"
        lineSize: tabs.lineSize
    }
    RelatedList {
        anchors { left: parent.left; right: parent.right; top: bar.bottom; bottom: parent.bottom; topMargin: Theme.spaceMd }
        visible: Nav.tab === "related"
    }
}
