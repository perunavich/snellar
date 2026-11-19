pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Flow {
    id: cards
    property var items: []
    spacing: Theme.spaceXl

    Repeater {
        model: cards.items
        delegate: Loader {
            id: slot
            required property var modelData
            sourceComponent: modelData.type === "artist" ? artist : modelData.type === "playlist" ? playlist : album
            Component { id: artist; ArtistCard { artist: slot.modelData } }
            Component { id: playlist; PlaylistCard { playlist: slot.modelData } }
            Component { id: album; AlbumCard { album: slot.modelData } }
        }
    }
}
