import QtQuick
import Snellar

Item {
    id: band
    property string label
    property real value: 0
    property real range: 12
    readonly property bool changed: value !== 0
    readonly property bool hot: area.containsMouse || area.pressed
    readonly property real half: (Theme.eqHeight - Theme.knob) / 2
    readonly property real zero: Theme.eqHeight / 2
    readonly property real knobY: zero - value / range * half
    signal moved(real v)

    implicitWidth: Theme.eqPre
    implicitHeight: Theme.eqHeight + Theme.spaceSm + caption.implicitHeight

    function db(v) { return (v > 0 ? "+" : v < 0 ? "−" : "") + Math.abs(v).toFixed(v % 1 ? 1 : 0) }

    Rectangle {
        id: track
        anchors.horizontalCenter: parent.horizontalCenter
        width: Theme.scrubTrack
        height: Theme.eqHeight
        radius: width / 2
        color: Theme.fillStrong
    }

    Rectangle {
        x: track.x
        y: Math.min(band.knobY, band.zero)
        width: track.width
        height: Math.abs(band.knobY - band.zero)
        visible: band.changed
        radius: width / 2
        color: Theme.accent
    }

    Rectangle {
        anchors.centerIn: knob
        width: Theme.knob + 2 * Theme.spaceXs
        height: width
        radius: width / 2
        visible: band.hot
        color: Theme.fillPress
    }

    Rectangle {
        id: knob
        x: (band.width - width) / 2
        y: band.knobY - height / 2
        width: Theme.knob
        height: width
        radius: width / 2
        color: band.changed ? Theme.accent : Theme.text
    }

    Text {
        anchors { horizontalCenter: knob.horizontalCenter; bottom: knob.top; bottomMargin: Theme.spaceXs }
        visible: band.hot
        text: qsTr("%1 dB").arg(band.db(band.value))
        color: Theme.text
        font.family: Theme.fontMono
        font.pixelSize: Theme.fsCaption
    }

    Text {
        id: caption
        anchors { bottom: parent.bottom; horizontalCenter: parent.horizontalCenter }
        text: band.label
        color: band.hot ? Theme.text : Theme.text3
        font.pixelSize: Theme.fsCaption
        font.weight: Font.Medium
    }

    MouseArea {
        id: area
        width: parent.width
        height: Theme.eqHeight
        hoverEnabled: true
        preventStealing: true
        cursorShape: Qt.PointingHandCursor
        function apply(y) {
            const v = (band.zero - y) / band.half * band.range
            band.moved(Math.round(Math.max(-band.range, Math.min(band.range, v)) * 2) / 2)
        }
        onPressed: m => apply(m.y)
        onPositionChanged: m => { if (pressed) apply(m.y) }
        onDoubleClicked: band.moved(0)
    }
}
