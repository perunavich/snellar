import QtQuick
import Snellar

SavedGrid {
    method: "savedAlbums"
    name: "albums"
    options: [{ key: "added", label: qsTr("Recently added") }, { key: "title", label: qsTr("Title") }, { key: "artist", label: qsTr("Artist") }, { key: "year", label: qsTr("Year") }]
    title: qsTr("Albums")
    emptyIcon: "disc"
    emptyText: qsTr("Save albums to see them here")
    card: AlbumCard { required property var modelData; album: modelData }
}
