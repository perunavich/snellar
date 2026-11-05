pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

ListView {
    id: list
    property int dropIndex: -1
    property Item held: null
    property real pointerY: 0
    property real grabY: 0
    readonly property int rowHeight: Theme.rowHeight
    readonly property bool dragging: dropIndex >= 0
    readonly property real edge: rowHeight / 2
    readonly property real maxY: Math.max(originY, originY + contentHeight - height)

    function target(y) {
        return Math.max(0, Math.min(Api.queue.length - 1, Math.round(y / rowHeight)))
    }

    function place() {
        const slot = held.parent
        held.y = pointerY + contentY - grabY - slot.y
        dropIndex = target(slot.y + held.y)
    }

    function follow() {
        if (dragging || Api.cursor < 0 || Api.cursor >= count) return
        positionViewAtIndex(Api.cursor, ListView.Contain)
        keptY = contentY
    }

    model: Api.queue
    property real keptY: 0
    property bool restoring: false
    onContentYChanged: if (!restoring) keptY = contentY
    onModelChanged: {
        dropIndex = -1
        held = null
        restoring = true
        restore.start()
    }
    Timer {
        id: restore
        interval: 0
        onTriggered: { list.contentY = list.keptY; list.restoring = false }
    }
    Connections {
        target: Api
        function onCursorChanged() { Qt.callLater(list.follow) }
    }
    onVisibleChanged: if (visible) Qt.callLater(follow)
    Component.onCompleted: Qt.callLater(follow)

    Timer {
        interval: 16
        repeat: true
        running: list.dragging && (list.pointerY < list.edge || list.pointerY > list.height - list.edge)
        onTriggered: {
            const step = list.pointerY < list.edge ? -(list.edge - list.pointerY) / 4 : (list.pointerY - list.height + list.edge) / 4
            list.contentY = Math.max(list.originY, Math.min(list.maxY, list.contentY + step))
            list.place()
        }
    }

    clip: true
    boundsBehavior: Flickable.StopAtBounds
    interactive: !list.dragging
    ScrollBar.vertical: ScrollBar { }

    Rectangle {
        visible: list.dropIndex >= 0
        z: 5
        y: list.dropIndex * list.rowHeight - list.contentY - 1
        width: list.width
        height: Theme.ring
        radius: Theme.ring / 2
        color: Theme.accent
    }

    delegate: Item {
        id: slot
        required property var modelData
        required property int index
        readonly property bool failed: !!Api.failure && Api.failure.tracks[modelData.id] !== undefined
        width: list.width
        height: list.rowHeight

        Item {
            id: lift
            width: parent.width
            height: parent.height
            z: list.held === lift ? 10 : 0
            Component.onDestruction: {
                if (list.held !== lift) return
                list.dropIndex = -1
                list.held = null
            }
            opacity: slot.index < Api.cursor ? 0.55 : 1

            Rectangle {
                anchors.fill: parent
                radius: Theme.rMd
                color: Theme.fillPress
                visible: list.held === lift && list.dragging
            }

            Item {
                id: handle
                width: Theme.gripWidth
                height: parent.height
                Icon { anchors.centerIn: parent; name: slot.failed ? "alert" : "grip"; size: Theme.iconSm; color: slot.failed ? Theme.red : Theme.text3 }
                MouseArea {
                    id: grip
                    anchors.fill: parent
                    preventStealing: true
                    cursorShape: Qt.SizeVerCursor
                    property real pressY: 0
                    onPressed: m => {
                        pressY = mapToItem(list, m.x, m.y).y
                        list.grabY = pressY - (slot.y - list.contentY)
                    }
                    onPositionChanged: m => {
                        list.pointerY = mapToItem(list, m.x, m.y).y
                        if (list.held !== lift && Math.abs(list.pointerY - pressY) < Theme.dragStart) return
                        list.held = lift
                        list.place()
                    }
                    onReleased: {
                        const to = list.dropIndex
                        list.dropIndex = -1
                        list.held = null
                        lift.y = 0
                        if (to >= 0 && to !== slot.index) Api.call("moveQueued", { from: slot.index, to: to, id: slot.modelData.id })
                    }
                    onCanceled: {
                        list.dropIndex = -1
                        list.held = null
                        lift.y = 0
                    }
                }
            }

            TrackRow {
                anchors { left: handle.right; right: parent.right }
                track: slot.modelData
                list: Api.queue
                rank: slot.index
                removable: true
                inQueue: true
                onRemoveRequested: Api.call("removeQueued", { index: slot.index, id: slot.modelData.id })
            }
        }
    }
}
