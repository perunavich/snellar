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

    TextMetrics { id: playWord; font.pixelSize: Theme.fsBody; font.weight: Font.DemiBold; text: qsTr("Play") }
    TextMetrics { id: pauseWord; font.pixelSize: Theme.fsBody; font.weight: Font.DemiBold; text: qsTr("Pause") }

    PillButton {
        id: play
        width: Theme.snap(Theme.iconSm + Theme.spaceSm + Math.max(playWord.advanceWidth, pauseWord.advanceWidth) + 2 * Theme.pillPadding)
        text: buttons.live && Api.playing ? qsTr("Pause") : qsTr("Play")
        icon: buttons.live && Api.playing ? "pause" : "play"
        primary: true
        enabled: buttons.tracks.length > 0
        onClicked: buttons.live ? Api.toggle() : Api.play(buttons.tracks, 0, buttons.from, buttons.origin)
    }
    ActionButton {
        text: qsTr("Shuffle")
        icon: "shuffle"
        enabled: buttons.tracks.length > 0
        onClicked: {
            if (buttons.ownShuffle) return buttons.shuffleRequested()
            Api.shufflePlay(buttons.tracks, buttons.from, buttons.origin)
        }
    }
}
