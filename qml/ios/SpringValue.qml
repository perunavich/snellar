import QtQuick


QtObject {
    id: v
    property real zeta: 1.0
    property real response: 0.4
    property real target: 0
    property real epsilon: 0.0005
    readonly property real value: s.x
    readonly property bool moving: loop.running
    property Spring s: Spring {}

    onZetaChanged: s.set(zeta, response)
    onResponseChanged: s.set(zeta, response)
    onTargetChanged: loop.running = true
    Component.onCompleted: { s.set(zeta, response); s.snap(target); }

    function snap(x) { s.snap(x); loop.running = false; }

    property FrameAnimation loop: FrameAnimation {
        id: loop
        running: false
        onTriggered: {
            const dt = Math.min(frameTime, 0.05);
            v.s.step(v.target, dt);
            if (v.s.done(v.target, v.epsilon)) { v.s.snap(v.target); running = false; }
        }
    }
}
