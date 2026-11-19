pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import QtQuick.Shapes
import Snellar

Item {
    id: sky
    property string part
    property bool moving: false
    readonly property var look: Theme.daypartLook(part)
    readonly property var art: Theme.daypartArt
    readonly property var body: look.sun || look.moon || null
    readonly property real bodyX: (body ? body.x : art.glow.x) * width
    readonly property real bodyY: (body ? body.y : art.glow.y) * height
    readonly property var stars: look.stars ? starsFor(width, height, look.stars) : []

    layer.enabled: true

    function mulberry(seed) {
        let s = seed
        return () => {
            s |= 0
            s = s + 0x6D2B79F5 | 0
            let t = Math.imul(s ^ s >>> 15, 1 | s)
            t = t + Math.imul(t ^ t >>> 7, 61 | t) ^ t
            return ((t ^ t >>> 14) >>> 0) / 4294967296
        }
    }

    function starsFor(w, h, spec) {
        const rnd = mulberry(art.starSeed), thin = mulberry(art.starSeed + 99), out = []
        const n = Math.round(w * h / art.starArea)
        for (let i = 0; i < n; i++) {
            const skip = spec.keep !== undefined && thin() > spec.keep
            const band = art.starBands[Math.floor(rnd() * art.starBands.length)]
            const x = rnd() * w, y = band[0] + rnd() * (band[1] - band[0]), r = 0.55 + rnd() * 0.55
            const period = Theme.twinkleMin + rnd() * Theme.twinkleSpan, phase = rnd(), a = spec.a * (0.45 + rnd() * 0.55)
            if (skip || (spec.top && y > art.starTop)) continue
            out.push({ x: x, y: y, r: r, period: period, phase: phase, a: a })
        }
        return out
    }

    component Glow: Item {
        id: glow
        property real cx
        property real cy
        property real r
        property real sx: 1
        property real sy: 1
        property color color
        property real alpha
        property var fall: Theme.daypartArt.fall
        property real breath: 0
        property int period: 0
        property bool moving: false
        readonly property real rest: 1 / (1 + breath)
        function stop(k) { return Qt.rgba(color.r, color.g, color.b, alpha * (1 + breath) * fall[k][1]) }

        anchors.fill: parent
        opacity: rest

        Shape {
            x: glow.cx - glow.r
            y: glow.cy - glow.r
            width: 2 * glow.r
            height: 2 * glow.r
            transform: Scale { origin.x: glow.r; origin.y: glow.r; xScale: glow.sx; yScale: glow.sy }
            ShapePath {
                strokeColor: "transparent"
                fillGradient: RadialGradient {
                    centerX: glow.r; centerY: glow.r; centerRadius: glow.r; focalX: glow.r; focalY: glow.r
                    GradientStop { position: glow.fall[0][0]; color: glow.stop(0) }
                    GradientStop { position: glow.fall[1][0]; color: glow.stop(1) }
                    GradientStop { position: glow.fall[2][0]; color: glow.stop(2) }
                    GradientStop { position: glow.fall[3][0]; color: glow.stop(3) }
                    GradientStop { position: glow.fall[4][0]; color: glow.stop(4) }
                    GradientStop { position: glow.fall[5][0]; color: glow.stop(5) }
                }
                PathAngleArc { centerX: glow.r; centerY: glow.r; radiusX: glow.r; radiusY: glow.r; startAngle: 0; sweepAngle: 360 }
            }
        }

        SequentialAnimation {
            running: glow.moving && glow.breath > 0
            loops: Animation.Infinite
            onRunningChanged: running ? glowSettle.stop() : glowSettle.start()
            NumberAnimation { target: glow; property: "opacity"; from: glow.rest; to: 1; duration: glow.period / 4; easing.type: Easing.OutSine }
            NumberAnimation { target: glow; property: "opacity"; from: 1; to: (1 - glow.breath) * glow.rest; duration: glow.period / 2; easing.type: Easing.InOutSine }
            NumberAnimation { target: glow; property: "opacity"; from: (1 - glow.breath) * glow.rest; to: glow.rest; duration: glow.period / 4; easing.type: Easing.InSine }
        }
        NumberAnimation { id: glowSettle; target: glow; property: "opacity"; to: glow.rest; duration: Motion.skySettle; easing.type: Easing.InOutSine }
    }

    component Sun: Item {
        id: sun
        property var spec
        property bool moving: false
        readonly property real r: spec.r * sky.height
        readonly property real reach: r * spec.cr
        readonly property real cx: spec.x * sky.width
        readonly property real cy: spec.y * sky.height
        readonly property real breath: Theme.daypartArt.coronaBreath
        property color core: spec.core
        property color limb: spec.limb
        property color rim: spec.rim
        property color glare: spec.corona
        function blend(t) { return Qt.rgba(core.r + (limb.r - core.r) * t, core.g + (limb.g - core.g) * t, core.b + (limb.b - core.b) * t, 1) }
        function halo(k) { return Qt.rgba(glare.r, glare.g, glare.b, spec.ca * (1 + breath) * k) }

        anchors.fill: parent

        Shape {
            id: corona
            x: sun.cx - sun.reach
            y: sun.cy - sun.reach
            width: 2 * sun.reach
            height: 2 * sun.reach
            opacity: 1 / (1 + sun.breath)
            ShapePath {
                strokeColor: "transparent"
                fillGradient: RadialGradient {
                    centerX: sun.reach; centerY: sun.reach; centerRadius: sun.reach
                    focalX: sun.reach; focalY: sun.reach; focalRadius: sun.r * 0.95
                    GradientStop { position: 0; color: sun.halo(1) }
                    GradientStop { position: 0.18; color: sun.halo(0.5) }
                    GradientStop { position: 0.45; color: sun.halo(0.16) }
                    GradientStop { position: 1; color: sun.halo(0) }
                }
                PathAngleArc { centerX: sun.reach; centerY: sun.reach; radiusX: sun.reach; radiusY: sun.reach; startAngle: 0; sweepAngle: 360 }
            }
            NumberAnimation { id: coronaSettle; target: corona; property: "opacity"; to: 1 / (1 + sun.breath); duration: Motion.skySettle; easing.type: Easing.InOutSine }
            SequentialAnimation on opacity {
                running: sun.moving
                loops: Animation.Infinite
                onRunningChanged: running ? coronaSettle.stop() : coronaSettle.start()
                NumberAnimation { to: 1; duration: Theme.coronaBreathing / 4; easing.type: Easing.OutSine }
                NumberAnimation { to: (1 - sun.breath) / (1 + sun.breath); duration: Theme.coronaBreathing / 2; easing.type: Easing.InOutSine }
                NumberAnimation { to: 1 / (1 + sun.breath); duration: Theme.coronaBreathing / 4; easing.type: Easing.InSine }
            }
        }

        Item {
            x: sun.cx - sun.r - 1
            y: sun.cy - sun.r - 1
            width: 2 * sun.r + 2
            height: 2 * sun.r + 2
            opacity: sun.spec.a
            layer.enabled: sun.spec.haze > 0
            layer.effect: MultiEffect { blurEnabled: true; blurMax: 2; blur: sun.spec.haze / 2 }

            Shape {
                anchors.fill: parent
                ShapePath {
                    strokeColor: Qt.rgba(sun.rim.r, sun.rim.g, sun.rim.b, 0.55)
                    strokeWidth: 1
                    fillGradient: RadialGradient {
                        centerX: sun.r + 1; centerY: sun.r + 1; centerRadius: sun.r; focalX: sun.r + 1; focalY: sun.r + 1
                        GradientStop { position: 0; color: sun.blend(0) }
                        GradientStop { position: 0.45; color: sun.blend(0.25) }
                        GradientStop { position: 0.85; color: sun.blend(0.75) }
                        GradientStop { position: 1; color: sun.blend(1) }
                    }
                    PathAngleArc { centerX: sun.r + 1; centerY: sun.r + 1; radiusX: sun.r - 0.5; radiusY: sun.r - 0.5; startAngle: 0; sweepAngle: 360 }
                }
            }
            Shape {
                anchors.fill: parent
                ShapePath {
                    strokeColor: "transparent"
                    fillGradient: RadialGradient {
                        centerX: sun.r + 1; centerY: sun.r + 1; centerRadius: sun.r / 2; focalX: sun.r + 1; focalY: sun.r + 1
                        GradientStop { position: 0; color: Qt.rgba(1, 1, 1, 0.7) }
                        GradientStop { position: 1; color: Qt.rgba(1, 1, 1, 0) }
                    }
                    PathAngleArc { centerX: sun.r + 1; centerY: sun.r + 1; radiusX: sun.r / 2; radiusY: sun.r / 2; startAngle: 0; sweepAngle: 360 }
                }
            }
        }
    }

    component Moon: Item {
        id: moon
        property var spec
        readonly property real r: spec.r * sky.height
        readonly property real side: Math.ceil(2 * r + 8)
        property color lit: spec.lit
        property color dim: spec.dim
        property color earth: spec.earth

        x: spec.x * sky.width - side / 2
        y: spec.y * sky.height - side / 2
        width: side
        height: side

        Shape {
            anchors.fill: parent
            ShapePath {
                strokeColor: "transparent"
                fillGradient: RadialGradient {
                    centerX: moon.side / 2; centerY: moon.side / 2; centerRadius: moon.r; focalX: moon.side / 2; focalY: moon.side / 2
                    GradientStop { position: 0; color: Qt.rgba(moon.earth.r, moon.earth.g, moon.earth.b, moon.spec.a * 0.10) }
                    GradientStop { position: 1; color: Qt.rgba(moon.earth.r, moon.earth.g, moon.earth.b, moon.spec.a * 0.16) }
                }
                PathAngleArc { centerX: moon.side / 2; centerY: moon.side / 2; radiusX: moon.r; radiusY: moon.r; startAngle: 0; sweepAngle: 360 }
            }
        }

        Item {
            id: cutter
            anchors.fill: parent
            visible: false
            layer.enabled: true
            layer.effect: MultiEffect { blurEnabled: true; blurMax: 4; blur: 0.7 }
            Rectangle {
                width: 2 * moon.r * moon.spec.k
                height: width
                radius: width / 2
                x: moon.side / 2 - moon.spec.dx * moon.r - width / 2
                y: moon.side / 2 + moon.spec.dy * moon.r - width / 2
                color: "black"
            }
        }

        Shape {
            anchors.fill: parent
            opacity: moon.spec.a
            layer.enabled: true
            layer.effect: MultiEffect { maskEnabled: true; maskInverted: true; maskSource: cutter; maskSpreadAtMin: 1; maskSpreadAtMax: 1 }
            ShapePath {
                strokeColor: "transparent"
                fillGradient: LinearGradient {
                    x1: moon.side / 2 + moon.r; y1: moon.side / 2 - moon.r / 2; x2: moon.side / 2 - moon.r; y2: moon.side / 2 + moon.r / 2
                    GradientStop { position: 0; color: moon.lit }
                    GradientStop { position: 1; color: moon.dim }
                }
                PathAngleArc { centerX: moon.side / 2; centerY: moon.side / 2; radiusX: moon.r; radiusY: moon.r; startAngle: 0; sweepAngle: 360 }
            }
        }
    }

    Rectangle { anchors.fill: parent; color: Theme.fillGroup }

    Rectangle {
        anchors.fill: parent
        opacity: sky.art.skyAlpha
        gradient: Gradient {
            GradientStop { position: 0; color: sky.look.sky[0] }
            GradientStop { position: 0.5; color: sky.look.sky[1] }
            GradientStop { position: 1; color: sky.look.sky[2] }
        }
    }

    Glow {
        visible: !!sky.look.far
        cx: sky.bodyX; cy: sky.bodyY; r: sky.art.far.r * sky.height; sx: sky.art.far.sx
        color: sky.look.far ? sky.look.far[0] : "transparent"
        alpha: sky.look.far ? sky.look.far[1] : 0
    }

    Repeater {
        model: sky.stars
        delegate: Rectangle {
            id: star
            required property var modelData
            x: modelData.x - modelData.r
            y: modelData.y - modelData.r
            width: 2 * modelData.r
            height: width
            radius: modelData.r
            color: sky.art.starColor
            opacity: modelData.a
            NumberAnimation { id: starSettle; target: star; property: "opacity"; to: star.modelData.a; duration: Motion.skySettle; easing.type: Easing.InOutSine }
            SequentialAnimation {
                running: sky.moving
                onRunningChanged: running ? starSettle.stop() : starSettle.start()
                PauseAnimation { duration: star.modelData.phase * star.modelData.period }
                SequentialAnimation {
                    loops: Animation.Infinite
                    NumberAnimation { target: star; property: "opacity"; to: star.modelData.a * (1 - sky.art.twinkle); duration: star.modelData.period / 2; easing.type: Easing.InOutSine }
                    NumberAnimation { target: star; property: "opacity"; to: star.modelData.a; duration: star.modelData.period / 2; easing.type: Easing.InOutSine }
                }
            }
        }
    }

    Rectangle {
        id: haze
        anchors.fill: parent
        visible: !!sky.look.haze
        property color tone: sky.look.haze ? sky.look.haze[0] : "transparent"
        readonly property real alpha: sky.look.haze ? sky.look.haze[1] : 0
        gradient: Gradient {
            GradientStop { position: 0; color: "transparent" }
            GradientStop { position: 0.7; color: Qt.rgba(haze.tone.r, haze.tone.g, haze.tone.b, 0) }
            GradientStop { position: 0.9; color: Qt.rgba(haze.tone.r, haze.tone.g, haze.tone.b, haze.alpha * 0.45) }
            GradientStop { position: 1; color: Qt.rgba(haze.tone.r, haze.tone.g, haze.tone.b, haze.alpha) }
        }
    }

    Glow {
        visible: !!sky.look.near
        cx: sky.bodyX; cy: sky.bodyY; r: sky.art.near.r * sky.height; sx: sky.art.near.sx; sy: sky.look.near && sky.look.near[2] ? sky.look.near[2] : 1
        color: sky.look.near ? sky.look.near[0] : "transparent"
        alpha: sky.look.near ? sky.look.near[1] : 0
        breath: sky.art.nearBreath
        period: Theme.nearBreathing
        moving: sky.moving
    }

    Loader {
        anchors.fill: parent
        active: !!sky.look.sun
        sourceComponent: Sun { spec: sky.look.sun; moving: sky.moving }
    }

    Loader {
        anchors.fill: parent
        active: !!sky.look.moon
        sourceComponent: Item { Moon { spec: sky.look.moon } }
    }

    Item {
        id: mist
        anchors.fill: parent
        visible: !!sky.look.mist && sky.look.mist[1] > 0
        Glow {
            cx: sky.art.mist.x * sky.width; cy: sky.art.mist.y * sky.height; r: sky.art.mist.r * sky.height; sx: sky.art.mist.sx
            color: sky.look.mist ? sky.look.mist[0] : "transparent"
            alpha: sky.look.mist ? sky.look.mist[1] : 0
            fall: sky.art.mistFall
        }
        NumberAnimation { id: mistSettle; target: mist; property: "x"; to: 0; duration: Motion.skySettle; easing.type: Easing.InOutSine }
        SequentialAnimation on x {
            running: sky.moving && mist.visible
            loops: Animation.Infinite
            onRunningChanged: running ? mistSettle.stop() : mistSettle.start()
            NumberAnimation { to: sky.art.mist.drift; duration: Theme.mistDrift / 4; easing.type: Easing.OutSine }
            NumberAnimation { to: -sky.art.mist.drift; duration: Theme.mistDrift / 2; easing.type: Easing.InOutSine }
            NumberAnimation { to: 0; duration: Theme.mistDrift / 4; easing.type: Easing.InSine }
        }
    }

    Rectangle {
        anchors.fill: parent
        visible: !!sky.look.wash
        gradient: Gradient {
            orientation: Gradient.Horizontal
            GradientStop { position: 0; color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, sky.look.wash || 0) }
            GradientStop { position: 1; color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0) }
        }
    }

    Glow {
        visible: !!sky.look.glow
        cx: sky.art.glow.x * sky.width; cy: sky.art.glow.y * sky.height; r: sky.art.glow.r * sky.height; sx: sky.art.glow.sx
        color: sky.look.glow ? sky.look.glow[0] : "transparent"
        alpha: sky.look.glow ? sky.look.glow[1] : 0
        breath: sky.art.glowBreath
        period: Theme.glowBreathing
        moving: sky.moving
    }

    Image {
        anchors.fill: parent
        source: "../../art/grain128.png"
        fillMode: Image.Tile
        smooth: false
        opacity: sky.art.grain
    }
}
