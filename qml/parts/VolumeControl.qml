import QtQuick
import Snellar

Row {
    id: volume
    property real barWidth: Theme.volumeBar
    property bool showBar: true
    property real buttonSize: Theme.controlMd
    spacing: Theme.spaceXs

    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        size: volume.buttonSize
        icon: Api.volume === 0 ? "mute" : (Api.volume < 40 ? "volumeLow" : "volume")
        tip: Api.volume === 0 ? qsTr("Unmute") : qsTr("Mute")
        onClicked: Api.toggleMute()
    }
    Scrubber {
        visible: volume.showBar && volume.barWidth > 0
        width: volume.barWidth
        anchors.verticalCenter: parent.verticalCenter
        value: Api.volume / 100
        onMoved: f => Api.setVolume(Math.round(f * 100))
    }

    WheelHandler {
        onWheel:e => Api.stepVolume(e.angleDelta.y / 120 * Api.volumeStep)
    }
}
