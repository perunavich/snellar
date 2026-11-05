import QtQuick
import QtQuick.Shapes
import Snellar

Item {
    id: icon

    property string name
    property color color: Theme.white
    property real size: Theme.iconMd
    property real stroke: 2
    property bool spinning: false
    property bool filled: icon.solid.indexOf(name) >= 0

    readonly property var solid: ["play", "pause", "next", "prev", "heartFill", "playSmall"]

    function circle(cx, cy, r) {
        return "M" + (cx - r) + " " + cy + "a" + r + " " + r + " 0 1 0 " + 2 * r + " 0a" + r + " " + r + " 0 1 0 " + -2 * r + " 0"
    }

    readonly property var paths: ({
        search: circle(11, 11, 7) + "M21 21L16.5 16.5",
        offline: "M1 1l22 22M16.72 11.06A10.94 10.94 0 0 1 19 12.55M5 12.55a10.94 10.94 0 0 1 5.17-2.39M10.71 5.05A16 16 0 0 1 22.58 9M1.42 9a15.91 15.91 0 0 1 4.7-2.88M8.53 16.11a6 6 0 0 1 6.95 0M12 20h.01",
        alert: circle(12, 12, 10) + "M12 8v4M12 16h.01",
        keyboard: "M4 6h16a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2zM6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8",
        close: "M18 6L6 18M6 6l12 12",
        droplet: "M12 2.69l5.66 5.66a8 8 0 1 1-11.31 0z",
        box: "M21 8l-9-5-9 5v8l9 5 9-5V8zM3 8l9 5 9-5M12 13v8",
        info: circle(12, 12, 10) + "M12 16v-4M12 8h.01",
        home: "M3 10.5L12 3l9 7.5V20a1 1 0 0 1-1 1h-5v-6h-6v6H4a1 1 0 0 1-1-1z",
        heart: "M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z",
        heartFill: "M20.84 4.61a5.5 5.5 0 0 0-7.78 0L12 5.67l-1.06-1.06a5.5 5.5 0 0 0-7.78 7.78l1.06 1.06L12 21.23l7.78-7.78 1.06-1.06a5.5 5.5 0 0 0 0-7.78z",
        disc: circle(12, 12, 10) + circle(12, 12, 3),
        folder: "M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z",
        history: "M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5M12 7v5l4 2",
        queue: "M3 6h13M3 12h13M3 18h9M17 15v6l5-3z",
        settings: circle(12, 12, 3) + "M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z",
        sliders: "M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M1 14h6M9 8h6M17 16h6",
        playlist: "M21 15V6M12 12H3M16 6H3M12 18H3" + circle(18.5, 15.5, 2.5),
        play: "M7 4.5v15a1 1 0 0 0 1.5.86l12.5-7.5a1 1 0 0 0 0-1.72L8.5 3.64A1 1 0 0 0 7 4.5z",
        playSmall: "M8 5v14l11-7z",
        pause: "M7 4h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1zM14 4h3a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1V5a1 1 0 0 1 1-1z",
        next: "M5 5.5v13a1 1 0 0 0 1.55.83L16 13v5a1 1 0 0 0 2 0V6a1 1 0 0 0-2 0v5L6.55 4.67A1 1 0 0 0 5 5.5z",
        prev: "M19 5.5v13a1 1 0 0 1-1.55.83L8 13v5a1 1 0 0 1-2 0V6a1 1 0 0 1 2 0v5l9.45-6.33A1 1 0 0 1 19 5.5z",
        shuffle: "M16 3h5v5M4 20L21 3M21 16v5h-5M15 15l6 6M4 4l5 5",
        repeat: "M17 1l4 4-4 4M3 11V9a4 4 0 0 1 4-4h14M7 23l-4-4 4-4M21 13v2a4 4 0 0 1-4 4H3",
        repeatOne: "M17 1l4 4-4 4M3 11V9a4 4 0 0 1 4-4h14M7 23l-4-4 4-4M21 13v2a4 4 0 0 1-4 4H3M11 10h1v4",
        volume: "M11 5L6 9H2v6h4l5 4zM15.54 8.46a5 5 0 0 1 0 7.07M19.07 4.93a10 10 0 0 1 0 14.14",
        volumeLow: "M11 5L6 9H2v6h4l5 4zM15.54 8.46a5 5 0 0 1 0 7.07",
        mute: "M11 5L6 9H2v6h4l5 4zM23 9l-6 6M17 9l6 6",
        lyrics: "M4 6h16M4 10h10M4 14h16M4 18h7",
        mic: "M12 1a3 3 0 0 0-3 3v8a3 3 0 0 0 6 0V4a3 3 0 0 0-3-3zM19 10v2a7 7 0 0 1-14 0v-2M12 19v4M8 23h8",
        more: circle(12, 5, 1) + circle(12, 12, 1) + circle(12, 19, 1),
        plus: "M12 5v14M5 12h14",
        back: "M15 18l-6-6 6-6",
        forward: "M9 18l6-6-6-6",
        down: "M6 9l6 6 6-6",
        up: "M18 15l-6-6-6 6",
        sort: "M21 16l-4 4-4-4M17 20V4M3 8l4-4 4 4M7 4v16",
        library: "M16 6l4 14M12 6v14M8 8v12M4 4v16",
        radio: circle(12, 12, 2) + "M16.24 7.76a6 6 0 0 1 0 8.49M7.76 16.24a6 6 0 0 1 0-8.49M19.07 4.93a10 10 0 0 1 0 14.14M4.93 19.07a10 10 0 0 1 0-14.14",
        refresh: "M21.5 2v6h-6M2.5 22v-6h6M2 11.5a10 10 0 0 1 18.8-4.3M22 12.5a10 10 0 0 1-18.8 4.2",
        check: "M20 6L9 17l-5-5",
        trash: "M3 6h18M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6M10 11v6M14 11v6M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2",
        download: "M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3",
        share: "M15 14l5-5-5-5M4 20v-7a4 4 0 0 1 4-4h12",
        music: "M9 18V5l12-2v13" + circle(6, 18, 3) + circle(18, 16, 3),
        user: circle(12, 7, 4) + "M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2",
        expand: "M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7",
        collapse: "M4 14h6v6M20 10h-6V4M14 10l7-7M3 21l7-7",
        grip: circle(9, 6, 1) + circle(15, 6, 1) + circle(9, 12, 1) + circle(15, 12, 1) + circle(9, 18, 1) + circle(15, 18, 1)
    })

    RotationAnimator {
        target: icon
        running: icon.spinning
        loops: Animation.Infinite
        from: 0
        to: 360
        duration: Theme.spin
        onRunningChanged: if (!running) icon.rotation = 0
    }

    implicitWidth: size
    implicitHeight: size
    layer.enabled: true
    layer.smooth: true
    layer.textureSize: Qt.size(size * 2, size * 2)

    Shape {
        anchors.centerIn: parent
        width: 24
        height: 24
        scale: icon.size / 24
        preferredRendererType: Shape.CurveRenderer

        ShapePath {
            strokeColor: icon.filled ? "transparent" : icon.color
            strokeWidth: icon.filled ? 0 : icon.stroke
            fillColor: icon.filled ? icon.color : "transparent"
            capStyle: ShapePath.RoundCap
            joinStyle: ShapePath.RoundJoin
            PathSvg { path: icon.paths[icon.name] || "" }
        }
    }
}
