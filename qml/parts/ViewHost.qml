pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: host

    readonly property var files: ({
        discover: "Discover", search: "Search", favorites: "Favorites", albums: "Albums",
        history: "History", settings: "Settings", album: "Album",
        artist: "Artist", playlist: "Playlist", discography: "Discography", songs: "Songs", artists: "Artists", library: "Library",
        "import": "Import"
    })

    property int serial: 0

    Connections {
        target: Nav
        function onViewChanged() { host.swap() }
        function onParamsChanged() { host.swap() }
    }

    Component.onCompleted: {
        Nav.host = host
        swap()
    }

    function scroller(item) {
        const queue = item ? [item] : []
        while (queue.length) {
            const next = queue.shift()
            if (next.contentY !== undefined && next.flickableDirection !== undefined) return next
            for (const child of next.children) queue.push(child)
        }
        return null
    }

    function memoryOf(view) { return view && view.remember ? view.remember() : null }
    function recallIn(view, memory) { if (view.recall) view.recall(memory) }

    function capture() {
        const page = serial % 2 ? one : two
        const view = page.item
        const flick = scroller(view)
        return { y: flick ? flick.contentY : 0, top: !flick || flick.contentY <= flick.originY, memory: memoryOf(view) }
    }

    function swap() { Qt.callLater(show) }

    function show() {
        serial++
        const next = serial % 2 ? one : two
        const prev = serial % 2 ? two : one
        next.params = Nav.params
        next.restore = Nav.restoring
        Nav.restoring = null
        next.source = "../views/" + (files[Nav.view] || "Discover") + ".qml"
        next.z = 1
        prev.z = 0
        next.state = "in"
        prev.state = "out"
    }

    component Page: Loader {
        id: page
        property var params: ({})
        property var restore: null
        property int tries: 0
        onLoaded: {
            if (!restore) return
            if (restore.memory) host.recallIn(item, restore.memory)
            if (restore.top) restore = null
            else {
                tries = 0
                settle.start()
            }
        }
        Timer {
            id: settle
            interval: 16
            repeat: true
            onTriggered: {
                const flick = host.scroller(page.item)
                if (!flick || !page.restore) return stop()
                const room = flick.originY + flick.contentHeight - flick.height
                if (room < page.restore.y && ++page.tries <= Theme.restoreTries) return
                const y = Math.max(flick.originY, Math.min(page.restore.y, room))
                if (flick.keptY !== undefined) flick.keptY = y
                flick.contentY = y
                page.restore = null
                stop()
            }
        }
        anchors.fill: parent
        opacity: 0
        visible: opacity > 0.01
        enabled: state !== "out"
        Binding { target: page.item; property: "params"; value: page.params; when: page.item !== null }
        transform: Translate { id: shift; y: 0 }
        states: [
            State { name: "in"; PropertyChanges { shift.y: 0; page.opacity: 1 } },
            State { name: "out"; PropertyChanges { shift.y: -Theme.spaceSm; page.opacity: 0 } }
        ]
        transitions: [
            Transition {
                to: "in"
                SequentialAnimation {
                    PropertyAction { target: shift; property: "y"; value: 16 }
                    ParallelAnimation {
                        NumberAnimation { target: shift; property: "y"; duration: Theme.emphasized; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard }
                        NumberAnimation { target: page; property: "opacity"; duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard }
                    }
                }
            },
            Transition {
                to: "out"
                SequentialAnimation {
                    ParallelAnimation {
                        NumberAnimation { target: shift; property: "y"; duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard }
                        NumberAnimation { target: page; property: "opacity"; duration: Theme.fast; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard }
                    }
                    PropertyAction { target: page; property: "source"; value: "" }
                }
            }
        ]
    }

    Page { id: one }
    Page { id: two }
}
