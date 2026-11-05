import QtQuick
import Snellar

MediaCard {
    property var playlist
    readonly property string from: playlist ? "playlist:" + playlist.id : ""
    menuKind: "playlist"
    menuEntity: playlist
    url: playlist ? playlist.thumb : ""
    title: playlist ? playlist.name : ""
    subtitle: playlist ? qsTr("Playlist · %n song(s)", "", playlist.count) : ""
    playable: !!playlist && playlist.count > 0
    live: Api.playingFrom(from)
    onOpened: Nav.playlist(playlist.id)
    onPlayed: Api.call("playlist", { id: playlist.id }, Api.later(v => { if (v && v.tracks.length) Api.play(v.tracks, 0, from, Api.origin("playlist", playlist)) }))
}
