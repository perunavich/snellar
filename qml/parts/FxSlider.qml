import QtQuick
import Snellar

Item {
    id: slider
    property real value: 0
    property real from: 0
    property real to: 1
    property real neutral: 0
    property real step: 0.05
    readonly property real fraction: Math.max(0, Math.min(1, (value - from) / (to - from)))
    readonly property real home: (neutral - from) / (to - from)
    readonly property bool changed: Math.abs(value - neutral) > step / 2
    readonly property bool hot: area.containsMouse || area.pressed
    signal moved(real v)

    implicitHeight: Theme.controlLg

    function emit(x) {
        const f = Math.max(0, Math.min(1, x / width))
        moved(Number((Math.round((from + f * (to - from)) / step) * step).toFixed(3)))
    }

    Rectangle {
        id: track
        anchors.verticalCenter: parent.verticalCenter
        width: parent.width
        height: Theme.scrubTrack
        radius: height / 2
        color: Theme.fillStrong
    }

    Rectangle {
        anchors.verticalCenter: parent.verticalCenter
        x: slider.width * slider.home
        width: 1
        height: Theme.spaceMd
        visible: slider.home > 0 && slider.home < 1
        color: Theme.fillSweep
    }

    Rectangle {
        anchors.verticalCenter: parent.verticalCenter
        x: slider.width * Math.min(slider.fraction, slider.home)
        width: slider.width * Math.abs(slider.fraction - slider.home)
        height: track.height
        radius: height / 2
        visible: slider.changed
        color: Theme.accent
    }

    Rectangle {
        anchors.centerIn: knob
        width: Theme.knob + 2 * Theme.spaceXs
        height: width
        radius: width / 2
        visible: slider.hot
        color: Theme.fillPress
    }

    Rectangle {
        id: knob
        anchors.verticalCenter: parent.verticalCenter
        x: slider.width * slider.fraction - width / 2
        width: Theme.knob
        height: width
        radius: width / 2
        color: slider.changed ? Theme.accent : Theme.text
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        preventStealing: true
        cursorShape: Qt.PointingHandCursor
        onPressed: m => slider.emit(m.x)
        onPositionChanged: m => { if (pressed) slider.emit(m.x) }
        onDoubleClicked: slider.moved(slider.neutral)
    }
}
