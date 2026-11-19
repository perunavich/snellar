import QtQuick
import Snellar

Timer {
    id: intent
    required property Item target
    property bool hovered: false
    property var page: null
    property point at

    interval: Theme.intentDwell
    running: hovered && !!page && !!page.id
    onRunningChanged: if (running) at = target.mapToItem(null, 0, 0)
    onTriggered: {
        const now = target.mapToItem(null, 0, 0)
        if (now.x === at.x && now.y === at.y) return Api.warm(page)
        at = now
        restart()
    }
}
