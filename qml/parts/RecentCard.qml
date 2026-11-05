pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Loader {
    id: slot
    property var entry
    sourceComponent: ({ album: albumCard, remote: albumCard, playlist: playlistCard, artist: artistCard })[entry.kind] || trackCard

    Component { id: albumCard; AlbumCard { album: slot.entry.item } }
    Component { id: playlistCard; PlaylistCard { playlist: slot.entry.item } }
    Component { id: artistCard; ArtistCard { artist: slot.entry.item } }
    Component { id: trackCard; TrackCard { track: slot.entry.item } }
}
