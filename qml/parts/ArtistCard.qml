import QtQuick
import Snellar

MediaCard {
    property var artist
    menuKind: "artist"
    menuEntity: artist
    warms: artist ? { method: "artist", id: artist.id } : null
    url: artist ? artist.thumb : ""
    title: artist ? artist.name : ""
    subtitle: artist ? Words.audience(artist.subtitle) : ""
    round: true
    playable: false
    onOpened: Nav.artist(artist.id)
}
