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

    function open(at, x, y) {
        if (!entity) return
        active = true
        const menu = item as EntityMenu
        menu.popup(at, x, y)
    }

    sourceComponent: EntityMenu {
        kind: spot.kind
        entity: spot.entity
        queueIndex: spot.queueIndex
        playlistId: spot.playlistId
    }
}
