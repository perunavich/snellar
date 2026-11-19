pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import QtQuick.Templates as T
import Snellar

Item {
    id: morph
    property Item content: null
    property T.Popup popup: null
    property Pressable wanted: null
    property Pressable opener: null
    property real radius: Theme.rLg
    property color fill: Theme.menu
    property bool open: false
    property bool moving: false
    property bool live: false
    property real progress: 0
    property real peak: 0
    property real blurPx: 0
    property rect source: Qt.rect(0, 0, 0, 0)
    property bool growUp: false
    property bool growLeft: false
    property Item nest: null
    property bool modal: false
    property Item origin: null
    property Item attach: null
    property bool seeded: false
    property real fade: 1
    property real scrimLevel: 0
    property point scrimAt: Qt.point(0, 0)

    readonly property bool shown: moving || progress > 0
    readonly property bool sourced: opener !== null || seeded

    property var s1: spring(Motion.openResponse, Motion.openDamping)
    property var s2: spring(Motion.openResponse, Motion.openDamping)
    property var sk: spring(Motion.kickResponse, Motion.kickDamping)
    property var cx: spring(Motion.openResponse, 1)
    property var cy: spring(Motion.openResponse, 1)
    property var sr: spring(Motion.radiusResponse, 1)
    property var st: spring(Motion.ejectResponse, Motion.ejectDamping)
    property var sc: spring(Motion.scrimIn, 1)
    property var form: ({ fw: 1, fh: 1, kx: 0, ky: 0, wait: 0, next: -1 })

    function spring(response, damping) {
        return { x: 0, v: 0, t: 0, r: response, z: damping, unit: 1, on: false }
    }

    function aim(s, target) {
        s.t = target
        s.on = true
    }

    function set(s, value) {
        s.x = value
        s.v = 0
        s.t = value
        s.on = false
    }

    function advance(s, dt) {
        if (!s.on) return false
        const r = Motion.solve(s.x - s.t, s.v, s.r, s.z, dt)
        if (Math.abs(r[0]) * s.unit < Motion.restPx && Math.abs(r[1]) * s.unit < Motion.restSpeedPx) {
            set(s, s.t)
            return false
        }
        s.x = s.t + r[0]
        s.v = r[1]
        return true
    }

    function lerp(a, b, t) { return a + (b - a) * t }
    function clamp01(v) { return Math.max(0, Math.min(1, v)) }

    function axes() {
        return [form.fw * s1.x + (1 - form.fw) * s2.x + form.kx * sk.x + cx.x,
                form.fh * s1.x + (1 - form.fh) * s2.x + form.ky * sk.x + cy.x]
    }

    function box() {
        const a = axes()
        const left = lerp(source.x, 0, a[0])
        const top = lerp(source.y, 0, a[1])
        const right = lerp(source.x + source.width, width, a[0])
        const bottom = lerp(source.y + source.height, height, a[1])
        return { left: left, top: top, right: right, bottom: bottom, w: Math.max(0, right - left), h: Math.max(0, bottom - top), p: Math.max(0, Math.min(a[0], a[1])) }
    }

    function home(b) {
        const slack = Motion.handoverPx
        return b.w - source.width < slack && b.h - source.height < slack && Math.abs(b.left - source.x) < slack && Math.abs(b.top - source.y) < slack
    }

    function retarget(fw, fh, kx, ky, target, response, damping, delay) {
        const before = axes()
        form.fw = fw
        form.fh = fh
        form.kx = kx
        form.ky = ky
        for (const s of [s1, s2, cx, cy]) s.r = response
        s1.z = damping
        s2.z = damping
        aim(s1, target)
        form.wait = delay
        form.next = target
        if (kx !== 0 || ky !== 0) {
            sk.v += 1
            sk.on = true
        }
        const after = axes()
        cx.x += before[0] - after[0]
        cy.x += before[1] - after[1]
        aim(cx, 0)
        aim(cy, 0)
    }

    function locate() {
        if (opener) source = opener.mapToItem(morph, 0, 0, opener.width, opener.height)
        else if (modal && origin) source = origin.mapToItem(morph, 0, 0, origin.width, origin.height)
        else if (modal) source = Qt.rect(width * (1 - Motion.modalFrom) / 2, height * (1 - Motion.modalFrom) / 2, width * Motion.modalFrom, height * Motion.modalFrom)
        else if (attach) source = point(attach.mapToItem(morph, 0, 0, attach.width, attach.height))
        else return
        growUp = source.y + source.height / 2 > height / 2
        growLeft = source.x + source.width / 2 > width / 2
    }

    function point(row) {
        const w = Math.min(width, Motion.pointSize)
        const h = Math.min(height, Motion.pointSize)
        const y = Math.max(0, Math.min(height - h, row.y + row.height / 2 - h / 2))
        return Qt.rect(row.x + row.width / 2 < width / 2 ? 0 : width - w, y, w, h)
    }

    function grow(from) {
        const fresh = !shown
        unpark()
        if (opener && opener !== from) opener.held = false
        opener = from
        seeded = !from && (modal || attach !== null)
        open = true
        if (from) from.held = true
        if (!sourced || Motion.reduced) return rest(1)
        locate()
        const reach = Math.max(1, width, height, Math.abs(source.x), Math.abs(source.y))
        for (const s of [s1, s2, cx, cy]) s.unit = reach
        sk.unit = reach * Math.max(Motion.openKickX, Motion.openKickY, Motion.closeKickX, Motion.closeKickY) * Motion.kickResponse
        sr.unit = Math.max(1, Math.min(width, height) / 2)
        st.unit = Math.max(1, Motion.merge)
        sc.unit = 255
        if (fresh) {
            for (const s of [s1, s2, sk, cx, cy, sr, sc]) set(s, 0)
            set(st, 1)
            peak = 0
        }
        if (modal) retarget(1, 1, 0, 0, 1, Motion.modalResponse, Motion.modalDamping, 0)
        else if (seeded) retarget(1, 1, 0, 0, 1, Motion.openResponse, Motion.openDamping, 0)
        else retarget(Motion.growRatio, Motion.growHeight(source.height, height), fresh ? Motion.openKickX : 0, fresh ? Motion.openKickY : 0, 1,
                      Motion.openResponse, Motion.openDamping, Motion.stepDelay * Motion.openResponse)
        aim(sr, 1)
        st.r = Motion.ejectResponse * Motion.tailScale()
        st.z = Motion.ejectDamping
        aim(st, 0)
        sc.r = Motion.scrimIn
        aim(sc, modal ? 1 : 0)
        moving = true
        apply()
    }

    function shrink() {
        if (!open) return
        open = false
        if (!sourced || Motion.reduced) return rest(0)
        const settled = s1.x > Motion.settledAt && s2.x > Motion.settledAt
        if (modal) retarget(1, 1, 0, 0, 0, Motion.modalCloseResponse, Motion.modalCloseDamping, 0)
        else if (seeded) retarget(1, 1, 0, 0, 0, Motion.closeResponse, Motion.closeDamping, 0)
        else retarget(Motion.shrinkRatio, Motion.jWidthRatio, settled ? -Motion.closeKickX : 0, settled ? -Motion.closeKickY : 0, 0,
                      Motion.closeResponse, Motion.closeDamping, Motion.stepDelay * Motion.closeResponse)
        aim(sr, 0)
        st.r = Motion.absorbResponse * Motion.tailScale()
        st.z = Motion.absorbDamping
        aim(st, 1)
        sc.r = Motion.scrimOut
        aim(sc, 0)
        moving = true
        apply()
    }

    function rest(at) {
        for (const s of [s1, s2, sr]) set(s, at)
        for (const s of [sk, cx, cy, st]) set(s, 0)
        set(sc, modal ? at : 0)
        form.next = -1
        moving = false
        peak = at
        open = at > 0
        if (at > 0) return apply()
        progress = 0
        scrimLevel = 0
        live = false
        unpark()
        if (opener) opener.held = false
    }

    function park() {
        if (!shown || !parent || !parent.parent || !parent.parent.parent) return
        nest = parent
        const above = nest.parent.parent
        const place = mapToItem(above, 0, 0)
        parent = above
        x = place.x
        y = place.y
        z = 1
    }

    function unpark() {
        if (!nest) return
        parent = nest
        x = 0
        y = 0
        z = -1
        nest = null
    }

    function step(dt) {
        const early = form.next >= 0 ? Math.min(dt, form.wait) : dt
        let any = advance(s2, early)
        if (form.next >= 0) {
            form.wait -= dt
            if (form.wait <= 0) {
                aim(s2, form.next)
                form.next = -1
                any = advance(s2, dt - early)
            } else any = true
        }
        for (const s of [s1, sk, cx, cy, sr, st, sc]) any = advance(s, dt) || any
        return any
    }

    function tick(dt) {
        if (dt <= 0) return
        if (!step(dt)) {
            moving = false
            if (!open) return rest(0)
        }
        apply()
    }

    function apply() {
        if (!sourced && !open) return rest(0)
        locate()
        const b = box()
        progress = b.p
        if (open) peak = Math.max(peak, b.p)
        if (!open && sourced && moving && home(b) && !sc.on) return rest(0)

        const round = Math.min(b.w, b.h) / 2
        const rr = Math.max(0, sr.x)
        const near = Math.min(round, lerp(round, radius, rr * rr))
        const far = Math.min(round, lerp(round, radius, rr))
        const merges = opener !== null && !modal
        const scale = opener ? opener.visualScale : 1
        const bodyW = merges ? source.width / 2 * scale : 0
        const bodyH = merges ? source.height / 2 * scale : 0
        const bodyX = source.x + source.width / 2
        const bodyY = source.y + source.height / 2
        const face = opener ? opener.faceColor : fill
        const blur = Motion.lite ? 0 : Motion.blur * Math.max(0, 1 - (open ? peak : b.p))
        blurPx = blur
        live = open && blur === 0 && b.p >= 1

        const kx = Math.min(1, b.w / Math.max(1, width))
        const ky = Math.min(1, b.h / Math.max(1, height))
        const mean = Math.sqrt(Math.max(1e-6, kx * ky))
        const cover = Math.max(kx, ky)
        const cw = modal ? width * cover : width * mean * Math.pow(Math.max(1e-6, kx / mean), Motion.contentStretch)
        const ch = modal ? height * cover : height * mean * Math.pow(Math.max(1e-6, ky / mean), Motion.contentStretch)
        const contentX = Math.round(modal ? (b.left + b.right - cw) / 2 : growLeft ? Math.min(b.right, width) - cw : Math.max(b.left, 0))
        const contentY = Math.round(modal ? (b.top + b.bottom - ch) / 2 : growUp ? Math.min(b.bottom, height) - ch : Math.max(b.top, 0))

        const reach = Motion.shadowReach
        const x0 = Math.floor(Math.min(b.left, bodyX - bodyW) - reach)
        const y0 = Math.floor(Math.min(b.top, bodyY - bodyH) - reach)
        liquid.x = x0
        liquid.y = y0
        liquid.width = Math.max(1, Math.ceil(Math.max(b.right, bodyX + bodyW) + reach) - x0)
        liquid.height = Math.max(1, Math.ceil(Math.max(b.bottom, bodyY + bodyH) + reach + Motion.shadowDrop) - y0)
        liquid.itemSize = Qt.size(liquid.width, liquid.height)
        liquid.shape = Qt.rect(b.left - x0, b.top - y0, b.w, b.h)
        liquid.radii = modal ? Qt.vector4d(far, far, far, far) : growUp ? Qt.vector4d(far, far, near, near) : Qt.vector4d(near, near, far, far)
        liquid.body = Qt.vector4d(bodyX - x0, bodyY - y0, bodyW, bodyH)
        liquid.bodyRadius = merges ? Math.min(opener.radius * scale, bodyW, bodyH) : 0
        liquid.bodyFill = Qt.vector4d(face.r, face.g, face.b, face.a)
        const turned = clamp01(progress / (modal ? Motion.modalFillSpan : Motion.fillSpan))
        const tint = opener ? Motion.blend(face, fill, turned) : fill
        liquid.fill = Qt.vector4d(tint.r, tint.g, tint.b, tint.a)
        liquid.edge = Qt.vector4d(Theme.frost.r, Theme.frost.g, Theme.frost.b, opener ? Motion.rimAlpha * turned : Motion.rimAlpha)
        liquid.contentRect = Qt.rect(contentX - x0, contentY - y0, cw, ch)
        liquid.feather = 1 + blur
        liquid.contentMix = live ? 0 : modal ? clamp01((progress - Motion.modalContentStart) / Motion.modalContentSpan) : Motion.lite ? Math.min(1, progress / Motion.contentAlphaSpan) : clamp01(progress)
        liquid.soft = lerp(Motion.shadowSoft, Motion.shadowWide, clamp01(Math.min(b.w, b.h) / Motion.shadowFull))
        const grown = clamp01(progress / Motion.mergeRamp)
        liquid.merge = merges && !Motion.lite ? Motion.merge * grown * clamp01(st.x) : 0
        liquid.shadow = Motion.shadowAlpha * grown
        fade = opener ? 1 : clamp01(progress / Motion.pointFade)
        scrimLevel = modal ? clamp01(sc.x) : 0
        scrimAt = mapFromItem(null, 0, 0)
    }

    onWidthChanged: if (shown) apply()
    onHeightChanged: if (shown) apply()
    onOpenerChanged: if (!opener && shown && !open) rest(0)

    Connections {
        target: morph.popup
        function onAboutToShow() { morph.grow(morph.wanted) }
        function onAboutToHide() {
            morph.shrink()
            morph.park()
        }
    }

    Connections {
        target: morph.parent
        enabled: morph.shown
        function onXChanged() { morph.apply() }
        function onYChanged() { morph.apply() }
    }

    Connections {
        target: morph.opener
        enabled: morph.shown && !morph.moving
        function onFaceColorChanged() { morph.apply() }
        function onVisualScaleChanged() { morph.apply() }
    }

    FrameAnimation {
        running: morph.moving
        onTriggered: morph.tick(Math.min(frameTime, Motion.maxStep) / Motion.timeScale)
    }

    ShaderEffectSource {
        id: grab
        visible: false
        sourceItem: morph.content
        sourceRect: morph.content ? Qt.rect(-morph.content.x, -morph.content.y, morph.width, morph.height) : Qt.rect(0, 0, 0, 0)
        hideSource: morph.shown && !morph.live
        live: morph.open && morph.sourced
        width: morph.width
        height: morph.height
    }

    MultiEffect {
        id: blurred
        width: morph.width
        height: morph.height
        source: grab
        visible: !morph.live
        autoPaddingEnabled: false
        blurEnabled: !Motion.lite
        blurMax: Motion.blurMax
        blur: Math.min(1, morph.blurPx / blurMax)
    }

    ShaderEffectSource {
        id: texture
        visible: false
        sourceItem: blurred
        hideSource: true
        live: !morph.live
        width: morph.width
        height: morph.height
    }

    Rectangle {
        objectName: "scrim"
        x: morph.scrimAt.x
        y: morph.scrimAt.y
        width: morph.Window.width
        height: morph.Window.height
        visible: morph.scrimLevel > 0
        color: Theme.scrim
        opacity: morph.scrimLevel
    }

    ShaderEffect {
        id: liquid
        property size itemSize: Qt.size(1, 1)
        property rect shape: Qt.rect(0, 0, 0, 0)
        property vector4d radii: Qt.vector4d(0, 0, 0, 0)
        property vector4d body: Qt.vector4d(0, 0, 0, 0)
        property vector4d bodyFill: Qt.vector4d(0, 0, 0, 0)
        property vector4d fill: Qt.vector4d(0, 0, 0, 0)
        property vector4d edge: Qt.vector4d(0, 0, 0, 0)
        property rect contentRect: Qt.rect(0, 0, 1, 1)
        property real merge: 0
        property real contentMix: 0
        property real shadow: 0
        property real soft: Motion.shadowSoft
        property real bodyRadius: 0
        property real bodyEdge: 0
        property real feather: 1
        property variant source: texture

        opacity: morph.fade
        visible: morph.shown
        vertexShader: "../../shaders/liquid.vert.qsb"
        fragmentShader: "../../shaders/liquid.frag.qsb"
    }

    ShaderEffectSource {
        visible: morph.opener !== null && morph.shown
        sourceItem: morph.opener ? morph.opener.face : null
        x: Math.round(morph.source.x)
        y: Math.round(morph.source.y)
        width: morph.source.width
        height: morph.source.height
        scale: morph.opener ? morph.opener.visualScale : 1
        opacity: morph.modal ? 1 - morph.clamp01(morph.progress / Motion.labelFade) : 1
    }
}
