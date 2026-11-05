pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

ListView {
    id: page
    property var tracks: []
    property Component intro
    property bool numbered: false
    property bool showAlbum: true
    property bool removable: false
    property bool radio: false
    property bool dated: false
    property bool reorderable: false
    property var playlistId: null
    property string from: ""
    property var origin: null
    readonly property real inset: Theme.space2xl
    signal removeRequested(var track)
    signal moveRequested(var track, int to)

    property var picked: ({})
    property int anchor: -1
    readonly property var selection: tracks.filter(t => picked[t.id] === true)

    function select(from, to, keep) {
        const next = keep ? Object.assign({}, picked) : {}
        for (let i = Math.min(from, to); i <= Math.max(from, to); i++) next[tracks[i].id] = true
        picked = next
    }

    function pick(index, modifiers) {
        if (modifiers & Qt.ShiftModifier && anchor >= 0) return select(anchor, index, modifiers & Qt.ControlModifier)
        anchor = index
        if (!(modifiers & Qt.ControlModifier)) return select(index, index, false)
        const next = Object.assign({}, picked)
        if (next[tracks[index].id]) delete next[tracks[index].id]
        else next[tracks[index].id] = true
        picked = next
    }

    function focusRow(index) {
        positionViewAtIndex(index, ListView.Contain)
        const slot = itemAtIndex(index)
        if (slot) focusIn(slot)
    }
    function focusIn(slot) { slot.rowItem.forceActiveFocus() }

    function keyOn(index, event) {
        const step = event.key === Qt.Key_Down ? 1 : event.key === Qt.Key_Up ? -1 : 0
        if (step && index + step >= 0 && index + step < count) {
            if (event.modifiers & Qt.ShiftModifier) {
                if (anchor < 0) anchor = index
                select(anchor, index + step, false)
            }
            focusRow(index + step)
        } else if (event.key === Qt.Key_A && event.modifiers & Qt.ControlModifier && count) {
            anchor = 0
            select(0, count - 1, false)
        } else if (event.key === Qt.Key_Escape && selection.length) {
            picked = {}
            anchor = -1
        } else return
        event.accepted = true
    }

    property int dropIndex: -1
    property Item held: null
    property int heldIndex: -1
    property real pointerY: 0
    property real grabY: 0
    readonly property bool dragging: dropIndex >= 0
    readonly property real edge: Theme.rowHeight / 2
    readonly property real maxY: Math.max(originY, originY + contentHeight - height)
    interactive: !dragging

    function place() {
        held.y = pointerY + contentY - grabY - held.parent.y
        dropIndex = Math.max(0, Math.min(count - 1, heldIndex + Math.round(held.y / Theme.rowHeight)))
    }

    Timer {
        interval: 16
        repeat: true
        running: page.dragging && (page.pointerY < page.edge || page.pointerY > page.height - page.edge)
        onTriggered: {
            const step = page.pointerY < page.edge ? -(page.edge - page.pointerY) / 4 : (page.pointerY - page.height + page.edge) / 4
            page.contentY = Math.max(page.originY, Math.min(page.maxY, page.contentY + step))
            page.place()
        }
    }

    function setRows() {
        const held = Window.activeFocusItem
        model = tracks
        if (tracks.length) Api.prefetch(tracks[0])
        if (held && !held.activeFocus) held.forceActiveFocus()
    }
    onTracksChanged: {
        anchor = -1
        setRows()
    }
    Component.onCompleted: setRows()
    property real keptY: 0
    property bool keptTop: true
    onKeptYChanged: keptTop = keptY <= originY
    onOriginYChanged: if (keptTop && !moving) contentY = originY
    property bool restoring: false
    onContentYChanged: if (!restoring) keptY = contentY
    onModelChanged: {
        currentIndex = -1
        restoring = true
        restore.start()
    }
    Timer {
        id: restore
        interval: 0
        onTriggered: {
            const room = Math.max(page.originY, page.originY + page.contentHeight + page.bottomMargin - page.height)
            page.keptY = page.keptTop ? page.originY : Math.max(page.originY, Math.min(room, page.keptY))
            page.contentY = page.keptY
            page.restoring = false
        }
    }
    clip: true
    reuseItems: true
    cacheBuffer: 800
    bottomMargin: inset
    boundsBehavior: Flickable.StopAtBounds

    ScrollBar.vertical: PageScrollBar { }

    header: Item {
        width: page.width
        height: head.height + page.inset + Theme.sectionGap

        Loader {
            id: head
            x: page.inset
            y: page.inset
            width: page.width - 2 * page.inset
            sourceComponent: page.intro
        }
    }

    delegate: Item {
        id: slot
        required property var modelData
        required property int index
        width: page.width
        height: row.height
        z: page.held === lift ? 10 : 0
        readonly property Item rowItem: row

        HoverHandler { id: over }

        Rectangle {
            visible: page.dropIndex === slot.index && page.heldIndex !== slot.index
            x: page.inset
            y: page.heldIndex < slot.index ? slot.height - 1 : -1
            width: page.width - 2 * page.inset
            height: Theme.ring
            radius: Theme.ring / 2
            color: Theme.accent
        }

        Item {
            id: lift
            width: parent.width
            height: parent.height
            Keys.onPressed: e => page.keyOn(slot.index, e)

            Rectangle {
                x: page.inset
                width: page.width - 2 * page.inset
                height: parent.height
                radius: Theme.rMd
                color: Theme.fillPress
                visible: page.held === lift && page.dragging
            }

            Item {
                width: page.inset
                height: parent.height
                visible: page.reorderable
                opacity: over.hovered || page.held === lift ? 1 : 0
                Icon { anchors.centerIn: parent; name: "grip"; size: Theme.iconSm; color: Theme.text3 }
                MouseArea {
                    anchors.fill: parent
                    preventStealing: true
                    cursorShape: Qt.SizeVerCursor
                    property real pressY: 0
                    onPressed: m => {
                        pressY = mapToItem(page, m.x, m.y).y
                        page.grabY = pressY - (slot.y - page.contentY)
                    }
                    onPositionChanged: m => {
                        page.pointerY = mapToItem(page, m.x, m.y).y
                        if (page.held !== lift && Math.abs(page.pointerY - pressY) < Theme.dragStart) return
                        page.held = lift
                        page.heldIndex = slot.index
                        page.place()
                    }
                    onReleased: {
                        const to = page.dropIndex
                        page.dropIndex = -1
                        page.held = null
                        page.heldIndex = -1
                        lift.y = 0
                        if (to >= 0 && to !== slot.index) page.moveRequested(slot.modelData, to)
                    }
                    onCanceled: {
                        page.dropIndex = -1
                        page.held = null
                        page.heldIndex = -1
                        lift.y = 0
                    }
                }
            }

            TrackRow {
                id: row
                x: page.inset
                width: page.width - 2 * page.inset
                track: slot.modelData
                list: page.tracks
                rank: slot.index
                numbered: page.numbered
                showAlbum: page.showAlbum
                removable: page.removable
                radio: page.radio
                dated: page.dated
                selected: page.picked[slot.modelData.id] === true
                group: selected && page.selection.length > 1 ? page.selection : []
                onPickRequested: modifiers => page.pick(slot.index, modifiers)
                playlistId: page.playlistId
                from: page.from
                origin: page.origin
                onRemoveRequested: page.removeRequested(slot.modelData)
            }
        }
    }
}
