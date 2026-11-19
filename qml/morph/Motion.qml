pragma Singleton
import QtQuick
import Snellar

QtObject {
    property real openResponse: 0.3
    property real openDamping: 0.8
    property real closeResponse: 0.3
    property real closeDamping: 0.8
    property real stepDelay: 0.1
    property real growRatio: 0.25
    property real jHeightRatio: 0.8
    property real maxJHeight: 200
    property real shrinkRatio: 0.9
    property real jWidthRatio: 0

    property real kickResponse: 0.35
    property real kickDamping: 0.8
    property real openKickX: 0
    property real openKickY: 0
    property real closeKickX: 0
    property real closeKickY: 0

    property real blur: 4
    property real blurMax: 32
    property real contentStretch: 0.25
    property real contentAlphaSpan: 0.65
    property bool lite: false

    property real merge: 22
    property real mergeRamp: 0.2
    property real tail: 0.5
    property real ejectResponse: 0.5
    property real ejectDamping: 0.75
    property real absorbResponse: 0.7
    property real absorbDamping: 0.8
    property real radiusResponse: 0.30
    property real fillSpan: 0.3
    property real rimAlpha: 0.10
    property real shadowAlpha: 0.32
    property real shadowReach: 44
    property real shadowDrop: 12
    property real shadowSoft: 4
    property real shadowWide: 16
    property real shadowFull: 120
    property real handoverPx: 1.5
    property real settledAt: 0.98

    property real smallSize: 44
    property real smallLift: 16
    property real largeSize: 160
    property real largeLift: 4
    property real smallPressDamping: 0.375
    property real smallPressResponse: 0.4
    property real largePressDamping: 0.6
    property real largePressResponse: 0.36
    property real trackingDamping: 0.625
    property real smallTrackingResponse: 0.262
    property real largeTrackingResponse: 0.314
    property real releaseScale: 1
    property real releaseSlack: 0.015
    property real settleResponse: 0.5
    property real settleDamping: 0.8
    property real settleKick: 0
    property real faceResponse: 0.2

    property real pointSize: 10
    property real pointFade: 0.25
    property real modalResponse: 0.42
    property real modalDamping: 0.92
    property real modalCloseResponse: 0.30
    property real modalCloseDamping: 1.0
    property real modalFillSpan: 0.35
    property real modalContentStart: 0.1
    property real modalContentSpan: 0.4
    property real modalFrom: 0.9
    property real labelFade: 0.2
    property real scrimIn: 0.25
    property real scrimOut: 0.22
    property real controlResponse: 0.30
    property real controlDamping: 0.82
    property real segmentDamping: 1.0
    property real switchStretch: 8
    property real tipResponse: faceResponse
    property real tipDamping: 1.0
    property real toastResponse: sheetOpenResponse
    property real toastDamping: sheetOpenDamping
    property real toastLeaveResponse: sheetCloseResponse
    property real toastLeaveDamping: sheetCloseDamping
    property real toastFrom: 0.92

    property real sheetOpenDamping: 0.845
    property real sheetOpenResponse: 0.531
    property real sheetCloseDamping: 1.0
    property real sheetCloseResponse: 0.336
    property real pageHandover: 0.2
    property real nowHandover: 0.5
    property real pausedCover: 0.9
    property real lyricAside: 0.9
    property real railResponse: sheetCloseResponse
    property real railDamping: sheetCloseDamping
    property real railWordSpan: 0.5
    property bool desktopReduced: false
    property int skyAwake: 20000
    property int skySettle: 1500
    readonly property bool reduced: Api.reduceMotion || desktopReduced
    property real trackDamping: 0.86
    property real trackResponse: 0.15

    property real timeScale: 1
    property real maxStep: 0.05
    property real restPx: 0.25
    property real restSpeedPx: 4

    function tailScale() { return 0.2 + 1.6 * tail }

    function railWords(fold, room) {
        const fade = Math.max(0, 1 - fold / railWordSpan)
        return room === undefined ? fade : fade * Math.max(0, Math.min(1, room / Theme.spaceSm))
    }

    function sizeMix(w, h) {
        return Math.max(0, Math.min(1, (Math.max(w, h) - smallSize) / (largeSize - smallSize)))
    }

    function pressScaleFor(w, h) {
        const s = Math.max(1, w, h)
        return (s + smallLift + (largeLift - smallLift) * sizeMix(w, h)) / s
    }

    function pressDampingFor(w, h) { return smallPressDamping + (largePressDamping - smallPressDamping) * sizeMix(w, h) }
    function pressResponseFor(w, h) { return smallPressResponse + (largePressResponse - smallPressResponse) * sizeMix(w, h) }
    function trackingResponseFor(w, h) { return smallTrackingResponse + (largeTrackingResponse - smallTrackingResponse) * sizeMix(w, h) }
    function releaseScaleFor(w, h) { return 1 - (1 - releaseScale) * Math.min(1, 40 / Math.max(1, w, h)) }

    function growHeight(from, to) {
        const travel = Math.max(1e-6, to - from)
        return Math.min(1, Math.min(jHeightRatio * travel, maxJHeight) / travel)
    }

    function blend(a, b, k) {
        const alpha = a.a + (b.a - a.a) * k
        if (alpha <= 0) return Qt.rgba(0, 0, 0, 0)
        const mix = (x, y) => (x * a.a + (y * b.a - x * a.a) * k) / alpha
        return Qt.rgba(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), alpha)
    }

    function solve(x0, v0, response, damping, dt) {
        const w = 2 * Math.PI / Math.max(0.02, response)
        const z = Math.min(1, Math.max(0.05, damping))
        if (z > 0.9995) {
            const e = Math.exp(-w * dt)
            const b = v0 + w * x0
            return [(x0 + b * dt) * e, (v0 - w * b * dt) * e]
        }
        const a = z * w
        const wd = w * Math.sqrt(1 - z * z)
        const e = Math.exp(-a * dt)
        const c = Math.cos(wd * dt)
        const s = Math.sin(wd * dt)
        const b = (v0 + a * x0) / wd
        return [e * (x0 * c + b * s), e * (v0 * c - (x0 * wd + a * b) * s)]
    }
}
