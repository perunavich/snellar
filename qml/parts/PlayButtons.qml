import QtQuick
import Snellar

Row {
    id: buttons
    property var tracks: []
    property string from: ""
    property var origin: null
    property bool ownShuffle: false
    signal shuffleRequested()
    readonly property bool live: Api.playingFrom(from, tracks)
    spacing: Theme.spaceMd

    PillButton {
        text: buttons.live && Api.playing ? qsTr("Pause") : qsTr("Play")
        icon: buttons.live && Api.playing ? "pause" : "play"
        primary: true
        enabled: buttons.tracks.length > 0
        onClicked: buttons.live ? Api.toggle() : Api.play(buttons.tracks, 0, buttons.from, buttons.origin)
    }
    PillButton {
        text: qsTr("Shuffle")
        icon: "shuffle"
        enabled: buttons.tracks.length > 0
        onClicked: {
            if (buttons.ownShuffle) return buttons.shuffleRequested()
            Api.call("shuffle", { on: true })
            Api.play(buttons.tracks, Math.floor(Math.random() * buttons.tracks.length), buttons.from, buttons.origin)
        }
    }
}
