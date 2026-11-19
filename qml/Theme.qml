pragma Singleton
import QtQuick
import Snellar

QtObject {
    property string accentName: Api.defaults.accent
    property string glass: Api.defaults.glass
    readonly property var desktop: Api.firstDesktop
    property string autoGlass: desktop.glass
    property string desktopName: desktop.desktop
    property bool frameless: desktop.frameless === true && Qt.platform.pluginName === "wayland"
    readonly property string glassLevel: ["glass", "cover", "solid"].indexOf(glass) >= 0 ? glass : autoGlass
    property bool navRight: Api.defaults.navRight
    property bool sidebarCollapsed: Api.defaults.sidebarCollapsed
    property real backgroundDim: Api.defaults.backgroundDim
    readonly property real wallpaperLumaUnknown: 1
    property real wallpaperLuma: wallpaperLumaUnknown
    property bool fieldWell: true
    property bool floorStrip: false

    signal restyle(var apply)

    property list<FontLoader> faces: [
        FontLoader { source: "../fonts/Commissioner.ttf" },
        FontLoader { source: "../fonts/MartianMono.ttf" }
    ]

    readonly property color ink: "#111115"
    readonly property color frost: "#eef1ee"
    readonly property color glassEdge: "#8fe6c8"
    readonly property color deepEdge: "#0b6a52"
    readonly property color mist: "#8a9694"
    readonly property color shade: "#050509"

    readonly property var palettes: ({
        glass: { accent: glassEdge, ink: ink },
        lagoon: { accent: "#81c8ff", ink: "#08111c" },
        teal: { accent: "#40d3c4", ink: "#07191a" },
        lilac: { accent: "#c7b8ff", ink: "#120d24" },
        coral: { accent: "#ffac99", ink: "#220c07" }
    })
    readonly property color accent: accentName === "cover" ? coverAccent : palettes[accentName].accent
    readonly property color accentInk: accentName === "cover" ? ink : palettes[accentName].ink
    readonly property color accentFill: Qt.rgba(accent.r, accent.g, accent.b, 0.12)
    readonly property real accentFloor: 0.53
    readonly property color coverAccent: {
        const c = Tone.average
        const l = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b
        const lift = l < 0.62 ? 0.62 / Math.max(l, 0.05) : 1
        const rgb = [c.r, c.g, c.b].map(x => Math.min(1, x * lift))
        const lin = x => x <= 0.04045 ? x / 12.92 : Math.pow((x + 0.055) / 1.055, 2.4)
        const whiter = k => rgb.map(x => x + (1 - x) * k)
        let k = 0
        while (k < 1) {
            const [r, g, b] = whiter(k)
            if (0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b) >= accentFloor) break
            k += 0.02
        }
        const [r, g, b] = whiter(Math.min(k, 1))
        return Qt.rgba(r, g, b, 1)
    }

    readonly property real coverBackdrop: 0.42
    readonly property real coverTint: 0.32 + 0.50 * coverBackdrop
    readonly property real coverTintLight: 0.46
    readonly property real coverTintDark: 0.88
    readonly property color solidLight: "#26262c"
    readonly property color solidDark: "#060609"
    readonly property bool dimmable: glassLevel !== "glass"
    readonly property real dimStep: 0.05
    readonly property real windowTint: glassLevel === "solid" ? 1 : glassLevel === "cover" ? dimmed(coverTintLight, coverTint, coverTintDark) : 0.32 + 0.50 * wallpaperLuma
    readonly property color window: glassLevel === "solid" ? solidWindow : Qt.rgba(shade.r, shade.g, shade.b, windowTint)
    readonly property color solidWindow: Qt.rgba(dimmed(solidLight.r, ink.r, solidDark.r), dimmed(solidLight.g, ink.g, solidDark.g), dimmed(solidLight.b, ink.b, solidDark.b), 1)
    readonly property color sidebar: Qt.rgba(0, 0, 0, 0.16)
    readonly property color chrome: Qt.rgba(ink.r, ink.g, ink.b, 0.48)
    readonly property color chromeStrong: Qt.rgba(ink.r, ink.g, ink.b, 0.86)
    readonly property color toastFill: Qt.rgba(ink.r, ink.g, ink.b, 0.96)
    readonly property color scrim: Qt.rgba(0, 0, 0, 0.34)
    readonly property color edgeTop: Qt.rgba(frost.r, frost.g, frost.b, 0.20)
    readonly property color hairline: Qt.rgba(frost.r, frost.g, frost.b, 0.09)

    readonly property color text: Qt.rgba(frost.r, frost.g, frost.b, 0.95)
    readonly property color text2: Qt.rgba(frost.r, frost.g, frost.b, 0.76)
    readonly property color text3: Qt.rgba(frost.r, frost.g, frost.b, 0.66)
    readonly property color separator: Qt.rgba(frost.r, frost.g, frost.b, 0.07)
    readonly property color fillGroup: Qt.rgba(frost.r, frost.g, frost.b, 0.06)
    readonly property color fillHover: Qt.rgba(frost.r, frost.g, frost.b, 0.08)
    readonly property color fillRaised: Qt.rgba(frost.r, frost.g, frost.b, 0.10)
    readonly property color fillPress: Qt.rgba(frost.r, frost.g, frost.b, 0.14)
    readonly property color fillStrong: Qt.rgba(frost.r, frost.g, frost.b, 0.16)
    readonly property color menu: Qt.tint(ink, fillRaised)
    readonly property color fillSweep: Qt.rgba(frost.r, frost.g, frost.b, 0.35)

    readonly property color toneDefault: "#202024"
    readonly property color shotTop: "#2b3a52"
    readonly property color shotBottom: "#4a3a5c"

    readonly property color white: "#ffffff"
    readonly property color red: "#ff453a"
    readonly property color danger: "#d70015"
    readonly property color coverShade: Qt.rgba(0, 0, 0, 0.5)
    readonly property color ambientTop: Qt.rgba(0.07, 0.07, 0.08, 0.12)
    readonly property color ambientBottom: Qt.rgba(0.07, 0.07, 0.08, 0.40)
    readonly property color bannerTop: Qt.rgba(0.1, 0.1, 0.11, 0.25)
    readonly property color navFill: Qt.rgba(0, 0, 0, 0.55)
    readonly property color navFillHover: Qt.rgba(0, 0, 0, 0.7)
    readonly property color pageBar: Qt.rgba(ink.r, ink.g, ink.b, 0.3)

    readonly property int rSm: 8
    readonly property int rMd: 12
    readonly property int rLg: 16
    readonly property int rPanel: 16
    readonly property int rRow: 12
    readonly property int rCard: 14
    readonly property int rArtS: 6
    readonly property int rArtM: 10
    readonly property int rArtL: 14

    readonly property string font: "Commissioner"
    readonly property string fontText: "Commissioner"
    readonly property string fontMono: "Martian Mono"
    readonly property int fsHero: 40
    readonly property int fsLargeTitle: 28
    readonly property int fsTitle1: 22
    readonly property int fsTitle2: 17
    readonly property int fsHeadline: 15
    readonly property int fsBody: 14
    readonly property int fsCallout: 13
    readonly property int fsCaption: 12
    readonly property int fsMicro: 11
    readonly property int fsLyric: 30
    readonly property int fsDisplay: 56
    readonly property real lyricDim: 0.7

    readonly property int windowWidth: 1440
    readonly property int windowHeight: 900
    readonly property int windowMinWidth: 960
    readonly property int windowMinHeight: 620
    readonly property int windowRadius: 12
    readonly property int windowEdge: 6
    readonly property int windowCorner: 14
    readonly property int sidebarWidth: 232
    readonly property int sidebarRow: controlMd
    readonly property int sidebarArt: 28
    readonly property int railWidth: sidebarRow + 2 * spaceMd
    readonly property int railBelow: sidebarWidth + capsuleWide + 2 * gap
    readonly property int fieldButton: 32
    readonly property int inspectorWidth: 340
    readonly property int compactBelow: 1180
    readonly property int capsuleHeight: 80
    readonly property int capsuleArt: 48
    readonly property int capsuleWide: 864
    readonly property int capsuleInfo: 180
    readonly property int capsuleCenter: 460
    readonly property int volumeBar: 72
    readonly property int volumeWide: 120
    readonly property int playPill: 48
    readonly property int gap: 16
    readonly property int dragStart: 6
    readonly property int navButton: 32
    readonly property int navBar: 2 * spaceLg + navButton
    readonly property int barBlur: 64
    readonly property real barSaturation: 0.2

    readonly property int spaceXs: 4
    readonly property int spaceSm: 8
    readonly property int spaceMd: 12
    readonly property int spaceLg: 16
    readonly property int spaceXl: 24
    readonly property int space2xl: 32
    readonly property int sectionGap: 28
    readonly property int lineGap: 2
    readonly property int ring: 2
    readonly property int pillPadding: 20

    readonly property int controlSm: 36
    readonly property int controlMd: 40
    readonly property int controlLg: 44
    readonly property int controlXl: 48
    readonly property int iconSm: 18
    readonly property int iconMd: 20
    readonly property int iconLg: 22
    readonly property int iconXl: 40
    readonly property int iconXs: 14
    readonly property int dotSize: 8
    readonly property int iconHero: 30
    readonly property real coverNoteShare: 0.36

    readonly property int rowHeight: 56
    readonly property int rowCover: 40
    readonly property int rowAlbumFrom: 640
    readonly property int rowDateFrom: 760
    readonly property int titleColumnMax: 560
    readonly property real titleShare: 0.55
    readonly property int cardCover: 168
    readonly property int settingsWidth: 640
    readonly property int settingsNavWidth: 176
    readonly property int settingsSplitMin: 640
    readonly property int settingTextMin: 200
    readonly property int componentStateMin: 96
    readonly property int confirmWidth: 380
    readonly property int heroCover: 200
    readonly property int heroGlyph: 72
    readonly property int editorWidth: 520
    readonly property int editorCover: 160
    readonly property int importWidth: 560
    readonly property int importBar: 6
    readonly property int keyHelpWidth: 860
    readonly property int toastText: 640
    readonly property int keyCap: 24
    readonly property int dateColumn: 112
    readonly property int timeColumn: 44
    readonly property int timeWidth: 40
    readonly property int gripWidth: 28
    readonly property int hitMin: 28
    readonly property int fieldWidth: 240
    readonly property int menuWidth: 224
    readonly property int menuWidthMax: 320
    readonly property int pickerRows: 6
    readonly property int segmentHeight: 32
    readonly property int segmentInset: 2
    readonly property int segmentNarrow: 200
    readonly property int segmentWidth: 320
    readonly property int segmentWide: 360
    readonly property int filterWidth: 440
    readonly property int searchFilterWidth: 528
    readonly property int switchWidth: 51
    readonly property int switchHeight: 31
    readonly property int knob: 14
    readonly property int scrubberHeight: 20
    readonly property int scrubTrack: 4
    readonly property int scrubGrow: 3
    readonly property int scrubStepPx: 2
    readonly property int scrollBar: 8
    readonly property int scrollThumb: 4
    readonly property int barsHeight: 14
    readonly property int barWidth: 3
    readonly property int cardText: 64
    readonly property int waveHeight: 220
    readonly property int waveButton: 64
    readonly property int waveItemMin: 176
    readonly property int waveItemMax: 2 * waveItemMin + spaceMd
    readonly property var daypartLooks: ({
        morning: { sky: ["#0b1230", "#1c2148", "#43304e"], far: ["#ffb4a0", 0.09], near: ["#ffe4cc", 0.16],
            sun: { x: 0.70, y: 0.90, r: 0.08, core: "#fff8ec", limb: "#ffd2b0", rim: "#ffc49c", haze: 0.7, corona: "#ffe2cc", ca: 0.30, cr: 3.2, a: 0.94 },
            stars: { a: 0.5, keep: 0.35, top: true }, haze: ["#c8a0b8", 0.08], mist: ["#b8a0c0", 0.08] },
        day: { sky: ["#0b2350", "#143a72", "#1c4c88"], far: ["#9fd0ff", 0.10], near: ["#fff6e0", 0.22, 0.4],
            sun: { x: 0.74, y: 0.83, r: 0.065, core: "#ffffff", limb: "#fff0d4", rim: "#ffe6b0", haze: 0, corona: "#fff8e8", ca: 0.42, cr: 3.0, a: 1 },
            haze: ["#8fc0ee", 0.06], mist: ["#cfe4f8", 0] },
        evening: { sky: ["#140822", "#340f30", "#661e24"], far: ["#ff7a40", 0.10], near: ["#ffb070", 0.20],
            sun: { x: 0.78, y: 1.0, r: 0.11, core: "#ffd6a0", limb: "#ff7434", rim: "#ffb070", haze: 0, corona: "#ff9a50", ca: 0.34, cr: 2.8, a: 0.97 },
            stars: { a: 0.5, keep: 0.25, top: true }, haze: ["#e0704a", 0.10], mist: ["#c06070", 0.10] },
        night: { sky: ["#03050d", "#08102a", "#142046"], far: ["#6f86d0", 0.10], near: ["#a9bcff", 0.16],
            moon: { x: 0.74, y: 0.86, r: 0.085, lit: "#f4f6ff", dim: "#c9d2ea", earth: "#8fa0c8", a: 0.80, dx: 0.42, dy: 0.22, k: 0.92 },
            stars: { a: 0.75 }, haze: ["#8090c0", 0.10], mist: ["#8090c0", 0.10] },
        wave: { sky: ["#111114", "#18181c", "#26262b"], wash: 0.10, glow: ["#8a8a96", 0.05] }
    })
    readonly property var daypartArt: ({
        skyAlpha: 0.94, grain: 0.03,
        far: { r: 1.2, sx: 2.5 }, near: { r: 0.45, sx: 1.3 }, glow: { x: 0.74, y: 0.86, r: 1.0, sx: 2.5 },
        mist: { x: 0.60, y: 0.97, r: 0.05, sx: 12, drift: 20 }, starColor: "#e9eeff", starArea: 8000,
        starBands: [[3, 20], [90, 102], [164, 214]], starTop: 104, starSeed: 7,
        nearBreath: 0.18, coronaBreath: 0.14, glowBreath: 0.15, twinkle: 0.55,
        fall: [[0, 1], [0.2, 0.82], [0.4, 0.55], [0.6, 0.28], [0.8, 0.09], [1, 0]],
        mistFall: [[0, 1], [0.35, 0.75], [0.7, 0.25], [1, 0], [1, 0], [1, 0]]
    })
    readonly property int nearBreathing: 32000
    readonly property int coronaBreathing: 26000
    readonly property int glowBreathing: 40000
    readonly property int mistDrift: 120000
    readonly property int twinkleMin: 9000
    readonly property int twinkleSpan: 13000
    readonly property int topHitWidth: 420
    readonly property int stateWidth: 400
    readonly property int topHitHeight: 104
    readonly property int topHitCover: 76
    readonly property real bannerRatio: 2.4
    readonly property int bannerMin: 260
    readonly property int bannerMax: 460
    readonly property real bannerShare: 0.45
    readonly property real bannerFocal: 0.25
    readonly property int bannerPx: 1920
    readonly property int readingWidth: 720
    readonly property real readingLine: 1.35
    readonly property int settingRow: 64
    readonly property int nowCover: 420
    readonly property int lyricSkeleton: 360
    readonly property int eqHeight: 208
    readonly property int eqScale: 34
    readonly property int eqPre: 44
    readonly property int audioWidth: 640
    readonly property int audioMinWidth: 520
    readonly property int fxLabel: 148
    readonly property int fxValue: 64
    readonly property int nowRise: 90

    readonly property int fast: 150
    readonly property int normal: 200
    readonly property int m3: 350
    readonly property int emphasized: 400
    readonly property int hover: 180
    readonly property int tipDelay: 600
    readonly property int toastTime: 3200
    readonly property int toastPerChar: 70
    readonly property int toastLongest: 12000
    readonly property int waitPatience: 4000
    readonly property int retryBase: 5000
    readonly property int retryLimit: 4
    readonly property int undoTime: 7000
    readonly property int restoreTries: 120
    readonly property real disabledOpacity: 0.4
    readonly property real playedOpacity: 0.75
    readonly property int ambientFade: 900
    readonly property int coverFade: 110
    readonly property int coverColumnPx: 640
    readonly property int coverNowPx: 800
    readonly property int coverCapsulePx: 112
    readonly property int coverRowPx: 96
    readonly property int ambientPx: 160
    readonly property int ambientDecode: 120
    readonly property int tonePx: 60
    readonly property int coversAhead: 2
    readonly property real standInBlur: 0.08
    readonly property int restyleFade: 350
    readonly property int offlineRetry: 5000
    readonly property int marqueePause: 1800
    readonly property int marqueeMsPerPx: 28
    readonly property int slide: 260
    readonly property int spin: 900
    readonly property int pulse: 800
    readonly property int sweep: 1100
    readonly property int fxDelay: 50
    readonly property int debounce: 150
    readonly property int recentSearches: 8
    readonly property int intentDwell: 300
    readonly property int lyricSettle: 3500
    readonly property int nudgeTick: 250

    readonly property var curveStandard: [0.2, 0, 0, 1, 1, 1]
    readonly property var curveHover: [0.1, 0.9, 0.2, 1, 1, 1]

    readonly property real lumaHigh: 0.70
    readonly property real lumaLow: 0.37

    function daypartLook(part) { return daypartLooks[part] || daypartLooks.wave }

    function washed(c) {
        const w = fillHover.a
        const a = c.a + w * (1 - c.a)
        const mix = (x, y) => (y * w + x * c.a * (1 - w)) / a
        return Qt.rgba(mix(c.r, frost.r), mix(c.g, frost.g), mix(c.b, frost.b), a)
    }

    function snap(v) { return Math.ceil(v / spaceXs - 0.001) * spaceXs }
    function iconFit(px) { return [iconXl, iconHero, iconLg, iconMd, iconSm].find(size => size <= px) || iconXs }

    function time(sec) {
        if (!isFinite(sec) || sec < 0) sec = 0
        const s = Math.floor(sec % 60), m = Math.floor(sec / 60) % 60, h = Math.floor(sec / 3600)
        const ss = s < 10 ? "0" + s : "" + s
        if (h > 0) return h + ":" + (m < 10 ? "0" + m : m) + ":" + ss
        return m + ":" + ss
    }

    function since(ms) {
        if (!(ms > 1e12)) return ""
        const minutes = Math.floor((Date.now() - ms) / 60000)
        if (minutes < 1) return qsTr("Just now")
        if (minutes < 60) return qsTr("%n minute(s) ago", "", minutes)
        if (minutes < 1440) return qsTr("%n hour(s) ago", "", Math.floor(minutes / 60))
        if (minutes < 10080) return qsTr("%n day(s) ago", "", Math.floor(minutes / 1440))
        return Qt.locale(Qt.uiLanguage).toString(new Date(ms), "d MMM yyyy")
    }

    function artists(track) {
        if (!track || !track.artists) return ""
        return track.artists.map(a => a.name).join(", ")
    }

    function dimmed(light, standard, dark) {
        const d = Math.max(0, Math.min(1, backgroundDim))
        return d < 0.5 ? light + (standard - light) * d * 2 : standard + (dark - standard) * (d - 0.5) * 2
    }

    function resized(url, tail) {
        if (!url) return ""
        const sized = url.replace(/=w\d+-h\d+[^&]*$/, tail)
        return Core.image_base && sized.startsWith("https") ? Core.image_base + encodeURIComponent(sized) : sized
    }

    function art(url, size) {
        const px = size || 544
        return resized(url, "=w" + px + "-h" + px + "-l90-rj")
    }

    function banner(url) { return resized(url, "=w" + bannerPx + "-h" + Math.round(bannerPx / bannerRatio) + "-p-l90-rj") }

    function bannerHeight(width, room) {
        const most = Math.max(bannerMin, Math.min(bannerMax, room * bannerShare))
        return Math.round(Math.max(bannerMin, Math.min(most, width / bannerRatio)))
    }
}
