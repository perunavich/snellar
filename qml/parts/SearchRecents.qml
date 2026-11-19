pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: recents
    property var entries: []
    spacing: Theme.spaceSm

    function load() {
        Api.call("getSetting", { key: "recentSearches" }, v => {
            if (recents) recents.entries = (Array.isArray(v) ? v : []).filter(e => e && (typeof e.query === "string" || (e.hit && e.hit.type))).slice(0, Theme.recentSearches)
        })
    }
    onVisibleChanged: if (visible) load()
    Component.onCompleted: if (visible) load()

    SectionTitle { visible: recents.entries.length > 0; text: qsTr("Recent searches") }

    Repeater {
        model: recents.entries
        delegate: Rectangle {
            id: row
            required property var modelData
            readonly property var hit: modelData.hit || null
            width: recents.width
            height: Theme.rowHeight
            radius: Theme.rMd
            color: hover.hovered ? Theme.fillHover : "transparent"

            Cover {
                id: art
                visible: !!row.hit
                x: Theme.spaceMd
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.rowCover
                height: Theme.rowCover
                radius: row.hit && row.hit.type === "artist" ? Theme.rowCover / 2 : Theme.rArtS
                px: Theme.coverRowPx
                url: row.hit ? row.hit.thumb : ""
            }
            Icon {
                visible: !row.hit
                x: Theme.spaceMd + (Theme.rowCover - Theme.iconMd) / 2
                anchors.verticalCenter: parent.verticalCenter
                name: "history"
                size: Theme.iconMd
                color: Theme.text2
            }
            Column {
                anchors { left: parent.left; leftMargin: Theme.rowCover + 2 * Theme.spaceMd; right: parent.right; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
                spacing: Theme.lineGap
                Text {
                    width: parent.width
                    text: row.hit ? (row.hit.title || row.hit.name) : row.modelData.query
                    color: Theme.text
                    font.pixelSize: Theme.fsBody
                    font.weight: Font.Medium
                    elide: Text.ElideRight
                }
                Text {
                    width: parent.width
                    visible: !!row.hit
                    text: row.hit ? Api.hitCaption(row.hit, true) : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsCallout
                    elide: Text.ElideRight
                }
            }
            HoverHandler { id: hover }
            TapHandler {
                gesturePolicy: TapHandler.ReleaseWithinBounds
                onTapped: row.hit ? Api.openHit(row.hit) : Nav.search(row.modelData.query)
            }
        }
    }

    EmptyState {
        width: parent.width
        visible: recents.entries.length === 0
        icon: "search"
        text: qsTr("Search for music")
        detail: qsTr("Type in the search field to find songs, artists, albums and playlists.")
    }
}
