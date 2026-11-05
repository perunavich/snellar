import QtQuick
import Snellar

Item {
    id: band
    property string label
    property real value: 0
    property real range: 12
    signal moved(real v)

    width: Theme.controlLg
    height: Theme.eqHeight

    readonly property real half: (track.height - Theme.spaceLg) / 2

    Text {
        anchors.horizontalCenter: parent.horizontalCenter
        text: (band.value > 0 ? "+" : "") + band.value.toFixed(1)
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono
    }

    Rectangle {
        id: track
        anchors { top: parent.top; topMargin: Theme.spaceXl; bottom: caption.top; bottomMargin: Theme.spaceSm; horizontalCenter: parent.horizontalCenter }
        width: Theme.eqTrack
        radius: width / 2
        color: Theme.fillRaised
        clip: true

        Rectangle {
            width: parent.width
            radius: width / 2
            color: Theme.accent
            opacity: 0.9
            readonly property real offset: band.value / band.range * band.half
            y: parent.height / 2 - Math.max(offset, 0)
            height: Math.abs(offset)
        }
        Rectangle {
            anchors { verticalCenter: parent.verticalCenter; horizontalCenter: parent.horizontalCenter }
            width: Theme.spaceMd
            height: Theme.ring
            color: Theme.text
            opacity: 0.4
        }
    }

    Text {
        id: caption
        anchors { bottom: parent.bottom; horizontalCenter: parent.horizontalCenter }
        text: band.label
        color: Theme.text2
        font.pixelSize: Theme.fsCaption
    }

    MouseArea {
        anchors.fill: track
        preventStealing: true
        cursorShape: Qt.PointingHandCursor
        function apply(y) {
            const v = (track.height / 2 - y) / band.half * band.range
            band.moved(Math.round(Math.max(-band.range, Math.min(band.range, v)) * 2) / 2)
        }
        onPressed: m => apply(m.y)
        onPositionChanged: m => { if (pressed) apply(m.y) }
        onDoubleClicked: band.moved(0)
    }
}
