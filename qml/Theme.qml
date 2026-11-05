pragma Singleton
import QtQuick
import Snellar

QtObject {
    property string accentName: Api.defaults.accent
    property string glass: Api.defaults.glass
    readonly property var desktop: Api.firstDesktop
    property string autoGlass: desktop.glass
    property string desktopName: desktop.desktop
    readonly property string glassLevel: ["glass", "cover", "solid"].indexOf(glass) >= 0 ? glass : autoGlass
    property bool navRight: Api.defaults.navRight
    readonly property real wallpaperLumaUnknown: 1
    property real wallpaperLuma: wallpaperLumaUnknown

    signal restyle(var apply)

    property list<FontLoader> faces: [
        FontLoader { source: "../fonts/Commissioner.ttf" },
        FontLoader { source: "../fonts/MartianMono.ttf" }
    ]

    readonly property color ink: "#0e1214"
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
    readonly property real backdropLuma: glassLevel === "cover" ? coverBackdrop : wallpaperLuma
    readonly property real windowTint: glassLevel === "solid" ? 1 : 0.32 + 0.50 * backdropLuma
    readonly property color window: glassLevel === "solid" ? ink : Qt.rgba(shade.r, shade.g, shade.b, windowTint)
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
    readonly property color black: "#000000"
    readonly property color red: "#ff453a"
    readonly property color coverShade: Qt.rgba(0, 0, 0, 0.5)
    readonly property color ambientTop: Qt.rgba(0.07, 0.07, 0.08, 0.12)
    readonly property color ambientBottom: Qt.rgba(0.07, 0.07, 0.08, 0.40)
    readonly property color bannerTop: Qt.rgba(0.1, 0.1, 0.11, 0.25)

    readonly property int rSm: 8
    readonly property int rMd: 12
    readonly property int rLg: 16
    readonly property int rPanel: 12
    readonly property int rRow: 8
    readonly property int rCard: 14
    readonly property int rSheet: 24
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
    readonly property int sidebarWidth: 232
    readonly property int sidebarRow: 32
    readonly property int sidebarArt: 24
    readonly property int inspectorWidth: 340
    readonly property int compactBelow: 1180
    readonly property int toolbarHeight: 52
    readonly property int capsuleHeight: 80
    readonly property int capsuleArt: 48
    readonly property int capsuleWide: 800
    readonly property int capsuleInfo: 180
    readonly property int capsuleCenter: 460
    readonly property int volumeBar: 72
    readonly property int volumeWide: 120
    readonly property int playPill: 48
    readonly property int gap: 16
    readonly property int dragStart: 6

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
    readonly property int iconHero: 30

    readonly property int rowHeight: 56
    readonly property int rowCover: 40
    readonly property int cardCover: 168
    readonly property int sheetWidth: 800
    readonly property int sheetHeight: 600
    readonly property int settingsWidth: 640
    readonly property int settingsNavWidth: 176
    readonly property int settingsSplitMin: 640
    readonly property int settingTextMin: 200
    readonly property int componentStateMin: 96
    readonly property int swatchCell: 72
    readonly property int confirmWidth: 380
    readonly property int heroCover: 200
    readonly property int editorWidth: 520
    readonly property int editorCover: 160
    readonly property int importWidth: 560
    readonly property int importBar: 6
    readonly property int keyHelpWidth: 760
    readonly property int toastText: 640
    readonly property int keyCap: 24
    readonly property int dateColumn: 112
    readonly property int timeColumn: 44
    readonly property int timeWidth: 40
    readonly property int gripWidth: 28
    readonly property int fieldWidth: 240
    readonly property int menuWidth: 224
    readonly property int segmentHeight: 32
    readonly property int segmentInset: 2
    readonly property int segmentNarrow: 200
    readonly property int segmentWidth: 320
    readonly property int segmentWide: 360
    readonly property int filterWidth: 440
    readonly property int switchWidth: 51
    readonly property int switchHeight: 31
    readonly property int knob: 14
    readonly property int scrubberHeight: 20
    readonly property int scrubTrack: 4
    readonly property int scrubGrow: 3
    readonly property int scrollBar: 8
    readonly property int scrollThumb: 4
    readonly property int barsHeight: 14
    readonly property int barWidth: 3
    readonly property int cardText: 58
    readonly property int waveHeight: 220
    readonly property int waveButton: 64
    readonly property int waveItem: 200
    readonly property int topHitWidth: 420
    readonly property int topHitHeight: 104
    readonly property int topHitCover: 76
    readonly property int artistBanner: 300
    readonly property int readingWidth: 720
    readonly property real readingLine: 1.35
    readonly property int settingRow: 64
    readonly property int nowCover: 420
    readonly property int lyricSkeleton: 360
    readonly property int eqHeight: 240
    readonly property int eqTrack: 28
    readonly property int sheetRise: 60
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
    readonly property int ambientFade: 900
    readonly property int restyleFade: 350
    readonly property int offlineRetry: 5000
    readonly property int marqueePause: 1800
    readonly property int marqueeMsPerPx: 28
    readonly property int slide: 260
    readonly property int spin: 900
    readonly property int pulse: 800
    readonly property int sweep: 1100
    readonly property int fxDelay: 120
    readonly property int debounce: 150
    readonly property int lyricSettle: 3500
    readonly property int nudgeTick: 250

    readonly property var springBase: [0.845, 0.531]
    readonly property var springCritical: [1.0, 0.336]
    readonly property var springTracking: [0.86, 0.15]

    readonly property var curveStandard: [0.2, 0, 0, 1, 1, 1]
    readonly property var curveJelly: [0.34, 1.56, 0.64, 1, 1, 1]
    readonly property var curveHover: [0.1, 0.9, 0.2, 1, 1, 1]

    readonly property real lumaHigh: 0.70
    readonly property real lumaLow: 0.37

    function snap(v) { return Math.ceil(v / spaceXs - 0.001) * spaceXs }

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

    function art(url, size) {
        if (!url) return ""
        const px = size || 544
        const sized = url.replace(/=w\d+-h\d+[^&]*$/, "=w" + px + "-h" + px + "-l90-rj")
        return Core.image_base && sized.startsWith("https") ? Core.image_base + encodeURIComponent(sized) : sized
    }
}
