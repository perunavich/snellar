import QtQuick
import Snellar

MediaCard {
    property var artist
    menuKind: "artist"
    menuEntity: artist
    url: artist ? artist.thumb : ""
    title: artist ? artist.name : ""
    subtitle: artist ? Words.subscribers(artist.subtitle) : ""
    round: true
    playable: false
    onOpened: Nav.artist(artist.id)
}
