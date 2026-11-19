import QtQuick
import Snellar

SavedGrid {
    method: "followed"
    name: "artists"
    options: [{ key: "added", label: qsTr("Recently added") }, { key: "title", label: qsTr("Name") }]
    title: qsTr("Artists")
    emptyIcon: "user"
    emptyText: qsTr("Follow artists to see them here")
    card: ArtistCard { required property var modelData; artist: modelData }
}
