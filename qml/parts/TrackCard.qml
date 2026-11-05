import QtQuick
import Snellar

MediaCard {
    id: card
    property var track
    url: track ? track.thumb : ""
    title: track ? track.title : ""
    subtitle: Theme.artists(track)
    menuKind: "track"
    menuEntity: track
    Timer { interval: Theme.debounce; running: card.hovered; onTriggered: Api.prefetch(card.track) }
    live: !!Api.current && !!track && Api.current.id === track.id
    onOpened: live ? Api.toggle() : Api.radio(track)
    onPlayed: Api.radio(track)
}
