pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Loader {
    id: spot
    property string kind: "track"
    property var entity
    property int queueIndex: -1
    property var playlistId: null
    active: false

    function toggle(at, x, y) {
        if (!entity) return
        active = true
        const menu = item as EntityMenu
        menu.toggle(at, x, y)
    }

    function context(at, x, y) {
        if (!entity) return
        active = true
        const menu = item as EntityMenu
        menu.context(at, x, y)
    }

    function heldBy(at) { return !!item && (item as EntityMenu).heldBy(at) }

    function close() { if (item) (item as EntityMenu).close() }

    sourceComponent: EntityMenu {
        kind: spot.kind
        entity: spot.entity
        queueIndex: spot.queueIndex
        playlistId: spot.playlistId
    }
}
