import QtQuick
import Snellar

MediaCard {
    property var album
    readonly property bool remote: !!album && album.kind === "Playlist"
    readonly property string from: album ? "album:" + album.id : ""
    menuKind: remote ? "remote" : "album"
    menuEntity: album
    url: album ? album.thumb : ""
    title: album ? album.title : ""
    subtitle: album ? [Words.kind(album.kind), album.year, Theme.artists(album)].filter(x => x).join(" · ") : ""
    live: Api.playingFrom(from) && (remote || (!!Api.current.album && Api.current.album.id === album.id))
    onOpened: remote ? Nav.remote(album.id) : Nav.album(album.id)
    onPlayed: Api.call(remote ? "remotePlaylist" : "album", { id: album.id }, Api.later(v => { if (v) Api.play(v.tracks, 0, from, Api.origin(remote ? "remote" : "album", album)) }))
}
