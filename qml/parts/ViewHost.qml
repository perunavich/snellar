pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: host

    readonly property var files: ({
        discover: "Discover", search: "Search", favorites: "Favorites", albums: "Albums",
        history: "History", downloaded: "Downloaded", downloadedSongs: "DownloadedSongs", settings: "Settings", album: "Album",
        artist: "Artist", playlist: "Playlist", discography: "Discography", songs: "Songs", artists: "Artists",
        "import": "Import"
    })

    property int serial: 0
    readonly property var current: (serial % 2 ? one : two).item

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
        next.wanted = true
        prev.wanted = false
    }

    component Page: Loader {
        id: page
        property var params: ({})
        property var restore: null
        property int tries: 0
        property bool wanted: false
        property real otherLevel: 0
        property real lift: 0
        readonly property real level: arrive.value
        onWantedChanged: {
            if (wanted) lift = Theme.spaceLg
            else if (level > Motion.settledAt) lift = 0
            if (!wanted && level <= 0) source = ""
        }
        onLevelChanged: if (!wanted && level <= 0) source = ""
        SpringValue {
            id: arrive
            reducible: true
            zeta: page.wanted ? Motion.sheetOpenDamping : Motion.sheetCloseDamping
            response: page.wanted ? Motion.sheetOpenResponse : Motion.sheetCloseResponse
            target: page.wanted && page.otherLevel <= Motion.pageHandover ? 1 : 0
        }
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
        opacity: Math.max(0, Math.min(1, level))
        visible: opacity > 0.01
        enabled: wanted
        Binding { target: page.item; property: "params"; value: page.params; when: page.item !== null }
        transform: Translate { y: page.lift * (1 - Math.min(1, page.level)) }
    }

    Page { id: one; otherLevel: two.level }
    Page { id: two; otherLevel: one.level }
}
