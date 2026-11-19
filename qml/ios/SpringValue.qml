import QtQuick
import Snellar

QtObject {
    id: v
    property real zeta: 1.0
    property real response: 0.4
    property real target: 0
    property real epsilon: 0.0005
    property real value: 0
    property real speed: 0
    property bool reducible: false
    readonly property bool moving: loop.running

    onTargetChanged: {
        if (reducible && Motion.reduced) snap(target)
        else loop.running = true
    }
    Component.onCompleted: snap(target)

    function snap(x) {
        value = x
        speed = 0
        loop.running = false
    }

    function kick(by) {
        speed += by
        loop.running = true
    }

    property FrameAnimation loop: FrameAnimation {
        running: false
        onTriggered: {
            const next = Motion.solve(v.value - v.target, v.speed, v.response, v.zeta, Math.min(frameTime, Motion.maxStep) / Motion.timeScale)
            if (Math.abs(next[0]) < v.epsilon && Math.abs(next[1]) < v.epsilon * 15) return v.snap(v.target)
            v.value = v.target + next[0]
            v.speed = next[1]
        }
    }
}
