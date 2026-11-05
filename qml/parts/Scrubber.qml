import QtQuick
import Snellar

Item {
    id: s
    property real value: 0
    property bool busy: false
    property bool dragging: false
    property real dragValue: 0
    signal moved(real f)
    signal committed(real f)

    implicitHeight: Theme.scrubberHeight

    SpringValue {
        id: pos
        zeta: Theme.springTracking[0]
        response: Theme.springTracking[1]
    }

    function follow(v, jump) {
        pos.target = v
        if (!jump) pos.snap(v)
    }

    onValueChanged: if (!dragging) follow(value, Math.abs(value - pos.value) > 0.03)
    onDragValueChanged: if (dragging) follow(dragValue, false)
    onDraggingChanged: follow(dragging ? dragValue : value, !dragging)
    SpringValue {
        id: grow
        zeta: 1.0
        response: 0.4
        target: s.dragging ? 1 : (area.containsMouse ? 0.7 : 0)
    }

    Rectangle {
        id: track
        anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter }
        height: Theme.scrubTrack + grow.value * Theme.scrubGrow
        radius: height / 2
        color: Theme.fillStrong
        clip: true

        Rectangle {
            width: parent.width * Math.max(0, Math.min(1, pos.value))
            height: parent.height
            radius: parent.radius
            color: Theme.accent
        }

        Rectangle {
            id: sweep
            visible: s.busy
            width: parent.width * 0.25
            height: parent.height
            radius: parent.radius
            color: Theme.fillSweep
            SequentialAnimation on x {
                running: s.busy && s.visible
                loops: Animation.Infinite
                NumberAnimation { from: -sweep.width; to: track.width; duration: Theme.sweep; easing.type: Easing.InOutSine }
            }
        }
    }

    Rectangle {
        width: Theme.knob
        height: Theme.knob
        radius: Theme.knob / 2
        color: Theme.accent
        x: track.width * Math.max(0, Math.min(1, pos.value)) - width / 2
        anchors.verticalCenter: parent.verticalCenter
        scale: grow.value
        visible: scale > 0.02
    }

    MouseArea {
        id: area
        anchors { fill: parent; topMargin: -Theme.spaceXs; bottomMargin: -Theme.spaceXs }
        hoverEnabled: true
        preventStealing: true
        cursorShape: Qt.PointingHandCursor

        function frac(x) { return Math.max(0, Math.min(1, x / width)) }

        onPressed: m => {
            s.dragValue = frac(m.x)
            pos.snap(s.dragValue)
            s.dragging = true
            s.moved(s.dragValue)
        }
        onPositionChanged: m => {
            if (!pressed) return
            s.dragValue = frac(m.x)
            pos.snap(s.dragValue)
            s.moved(s.dragValue)
        }
        onReleased: {
            s.committed(s.dragValue)
            s.dragging = false
        }
        onCanceled: s.dragging = false
    }
}
