pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

ApplicationWindow {
    id: win
    width: argv.size ? Number(argv.size.split("x")[0]) : Theme.windowWidth
    height: argv.size ? Number(argv.size.split("x")[1]) : Theme.windowHeight
    minimumWidth: Theme.windowMinWidth
    minimumHeight: Theme.windowMinHeight
    visible: true
    title: Api.current ? Api.current.title + " · " + Theme.artists(Api.current) : "Snellar"
    color: "transparent"
    flags: Theme.frameless ? (Qt.Window | Qt.FramelessWindowHint) : Qt.Window
    font.family: Theme.fontText
    font.pixelSize: Theme.fsBody
    palette.window: Theme.ink
    palette.windowText: Theme.text
    palette.base: Theme.menu
    palette.alternateBase: Theme.menu
    palette.text: Theme.text
    palette.button: Theme.menu
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.accentInk
    palette.placeholderText: Theme.text3

    readonly property var argv: {
        const out = {}
        for (const a of Qt.application.arguments) {
            const m = /^--([\w-]+)=(.*)$/.exec(a)
            if (m) out[m[1]] = m[2]
        }
        return out
    }
    readonly property bool compact: width < Theme.compactBelow
    readonly property real luma: Number(argv.luma)
    readonly property color backdropTop: argv.luma ? Qt.rgba(luma, luma, luma, 1) : Theme.shotTop
    readonly property color backdropBottom: argv.luma ? Qt.rgba(luma, luma, luma, 1) : Theme.shotBottom
    readonly property bool restyling: grabbing || fade.running
    property bool grabbing: false

    function restyle(apply) {
        grabbing = stage.grabToImage(shot => {
            grabbing = false
            before.source = shot.url
            apply()
            fade.restart()
        })
        if (!grabbing) apply()
    }

    Connections {
        target: Theme
        function onRestyle(apply) { win.restyle(apply) }
    }

    onActiveChanged: {
        if (active && !argv.luma) wallpaper.reload()
        if (active && !Api.mock) Core.refreshDesktop()
    }

    property bool closeToTray: false
    readonly property bool hidesOnClose: closeToTray && Api.trayReady
    readonly property var trayEntries: [
        { id: "window", label: win.visible ? qsTr("Hide Snellar") : qsTr("Show Snellar"), enabled: true },
        { id: "playPause", label: Api.playing ? qsTr("Pause") : qsTr("Play"), enabled: !!Api.current },
        { id: "next", label: qsTr("Next"), enabled: !!Api.current },
        ...(Api.updateReady ? [{ id: "restartUpdate", label: qsTr("Restart to update"), enabled: true }] : []),
        { separator: true },
        { id: "quit", label: qsTr("Quit Snellar"), enabled: true }
    ]

    function pushTray() { Core.tray(JSON.stringify({ on: closeToTray, title: "Snellar", entries: trayEntries })) }
    onTrayEntriesChanged: pushTray()
    onCloseToTrayChanged: pushTray()

    function bringUp() {
        show()
        raise()
        requestActivate()
    }

    function trayAction(action) {
        if (action === "window") {
            if (visible) hide()
            else bringUp()
        } else if (action === "raise") bringUp()
        else if (action === "playPause") Api.toggle()
        else if (action === "next") Api.next()
        else if (action === "restartUpdate") Api.restartForUpdate()
        else if (action === "quit") quit()
    }

    property bool quitting: false
    property bool quitEnds: true
    signal quitCalled()

    function quit() {
        quitting = true
        quitCalled()
        if (quitEnds) Qt.quit()
    }

    onClosing: close => {
        if (!hidesOnClose || quitting) return
        close.accepted = false
        hide()
    }

    Connections {
        target: Core
        function onTrayAction(action) { win.trayAction(action) }
    }

    Connections {
        target: Api
        function onSettingSaved(key, value) { if (key === "closeToBackground") win.closeToTray = !!value }
        function onTrayReadyChanged() { if (!Api.trayReady && !win.visible) win.bringUp() }
        function onDesktopInfoChanged() { if (!win.argv.luma && Theme.desktopName === "gnome") wallpaper.reload() }
    }

    Item {
        id: stage
        anchors.fill: parent
        layer.enabled: Theme.frameless && win.visibility === Window.Windowed
        layer.samplerName: "src"
        layer.effect: RoundMask { radius: Theme.windowRadius }

        TapHandler {
            acceptedButtons: Qt.BackButton | Qt.ForwardButton
            onTapped: (point, button) => button === Qt.BackButton ? Nav.back() : Nav.forward()
        }

        Rectangle {
            anchors.fill: parent
            visible: !!win.argv.shot
            gradient: Gradient {
                GradientStop { position: 0; color: win.backdropTop }
                GradientStop { position: 1; color: win.backdropBottom }
            }
        }

        Image {
            anchors.fill: parent
            visible: !!win.argv.backdrop
            source: visible ? "file://" + win.argv.backdrop : ""
        }

        Item {
            anchors.fill: parent
            visible: !(nowLoader.item && (nowLoader.item as NowPlaying).covering)
            opacity: nowLoader.item ? 1 - (nowLoader.item as NowPlaying).hiding : 1

            Rectangle { anchors.fill: parent; visible: Theme.glassLevel === "cover"; color: Theme.shade }
            Ambient {
                anchors.fill: parent
                visible: Theme.glassLevel === "cover"
                source: visible && Api.current ? Theme.art(Api.current.thumb, Theme.ambientPx) : ""
            }
            Rectangle { anchors.fill: parent; color: Theme.window }

            WindowDrag {
                anchors.fill: parent
                onDoubleTapped: frame.toggleMaximized()
            }

            Sidebar {
                id: sidebar
                anchors { left: parent.left; top: parent.top; bottom: parent.bottom }
                z: 1
                narrow: win.width < Theme.railBelow
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false
            }

            Item {
                id: content
                readonly property rect floorBox: Theme.floorStrip ? Qt.rect(0, height - Nav.floor, width, Nav.floor) : Qt.rect(capsule.x - x, capsule.y - y, capsule.width, capsule.height)
                readonly property real floorRadius: Theme.floorStrip ? 0 : capsule.radius
                anchors {
                    left: sidebar.right
                    right: win.compact ? parent.right : column.left
                    top: parent.top
                    bottom: parent.bottom
                }
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false

                ViewHost {
                    id: host
                    anchors.fill: parent
                    clip: true
                    layer.enabled: pageBar.solid > 0 || Nav.floor > 0
                    layer.samplerName: "source"
                    layer.effect: BarBlur { amount: pageBar.solid; floor: Nav.floor; floorAmount: pageBar.floorSolid; floorBox: content.floorBox; floorRadius: content.floorRadius }
                }

                BarShade {
                    x: content.floorBox.x
                    y: content.floorBox.y
                    width: content.floorBox.width
                    height: content.floorBox.height
                    radius: content.floorRadius
                    visible: Nav.floor > 0
                    opacity: pageBar.floorSolid
                    edgeOnTop: true
                    edge: Theme.floorStrip
                }

                PageBar {
                    id: pageBar
                    anchors { left: parent.left; right: parent.right; top: parent.top }
                    host: host
                    onDoubleTapped: frame.toggleMaximized()
                }
            }

            NowColumn {
                id: column
                anchors {
                    right: parent.right
                    top: parent.top
                    bottom: parent.bottom
                }
                width: Theme.inspectorWidth
                visible: !win.compact
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false
            }

            Binding { target: Nav; property: "floor"; value: win.compact ? Theme.capsuleHeight + 2 * Theme.gap : 0 }

            MouseArea {
                anchors.fill: capsule
                visible: capsule.visible
                hoverEnabled: true
                acceptedButtons: Qt.AllButtons
                onWheel: wheel => wheel.accepted = true
            }

            PlayerCapsule {
                id: capsule
                anchors {
                    left: content.left
                    right: content.right
                    bottom: parent.bottom
                    leftMargin: Theme.gap
                    rightMargin: Theme.gap
                    bottomMargin: Theme.gap
                }
                visible: win.compact
            }
        }

        Loader {
            id: nowLoader
            property bool shown: false
            anchors.fill: parent
            active: Nav.nowPlaying || shown
            source: "parts/NowPlaying.qml"

            Connections {
                target: nowLoader.item as NowPlaying
                function onVisibleChanged() { nowLoader.shown = (nowLoader.item as NowPlaying).visible }
            }
        }

        Loader {
            id: audioLoader
            readonly property NowPlaying now: Nav.nowPlaying ? nowLoader.item as NowPlaying : null
            property bool shown: false
            anchors.fill: parent
            active: Nav.audio || shown
            source: "parts/AudioPanel.qml"

            Connections {
                target: audioLoader.item as AudioPanel
                function onVisibleChanged() { audioLoader.keep() }
                function onSavingChanged() { audioLoader.keep() }
            }
            function keep() {
                const panel = item as AudioPanel
                shown = panel.visible || panel.saving
            }
        }

        Binding {
            when: audioLoader.item !== null
            target: audioLoader.item as AudioPanel
            property: "dock"
            value: audioLoader.now ? audioLoader.now.controls : win.compact ? capsule : column
        }
        Binding {
            when: audioLoader.item !== null
            target: audioLoader.item as AudioPanel
            property: "button"
            value: audioLoader.now ? audioLoader.now.soundButton : win.compact ? capsule.soundButton : column.soundButton
        }
        Binding {
            when: audioLoader.item !== null
            target: audioLoader.item as AudioPanel
            property: "settle"
            value: audioLoader.now ? audioLoader.now.p : 1
        }

        Column {
            id: banners
            readonly property bool overPage: !Nav.nowPlaying && !Nav.audio
            z: 5
            x: overPage ? content.x + pageBar.side : Theme.gap
            anchors { top: parent.top; topMargin: Theme.gap }
            width: overPage ? content.width - 2 * pageBar.side : parent.width - 2 * Theme.gap
            spacing: Theme.spaceSm

            Banner {
                anchors.horizontalCenter: parent.horizontalCenter
                active: !!Api.libraryProblem && !Api.libraryProblem.saving
                tint: Theme.red
                text: qsTr("Not saving: changes will be lost when you close Snellar")
                actionText: qsTr("Try again")
                onActed: Api.call("libraryRetry", {}, opened => { if (opened) Api.libraryChanged() })
            }
            Banner {
                id: reachBanner
                objectName: "reach"
                property string code: ""
                readonly property bool offline: code === "net.offline"
                anchors.horizontalCenter: parent.horizontalCenter
                active: Api.reach.length > 0
                icon: offline ? "offline" : "alert"
                text: offline ? qsTr("No internet connection. You can still play downloaded music.") : Errors.text(code)
                actionText: offline && Nav.view !== "downloaded" ? qsTr("Downloaded") : ""
                onActed: Nav.go("downloaded")

                Binding {
                    target: reachBanner
                    property: "code"
                    value: Api.reach
                    when: Api.reach.length > 0
                    restoreMode: Binding.RestoreNone
                }
            }
        }

        LibraryGate {
            anchors.fill: parent
            z: 6
        }

        NudgeHost {
            anchors {
                right: parent.right
                bottom: parent.bottom
                rightMargin: win.compact || Theme.navRight ? Theme.gap : Theme.inspectorWidth + 2 * Theme.gap
                bottomMargin: win.compact ? Theme.capsuleHeight + 2 * Theme.gap : Theme.gap
            }
            z: 2
        }

        TrackDrag {
            id: carried
            Component.onCompleted: Nav.carried = carried
        }

        Toast {
            id: toast
            Component.onCompleted: Nav.toast = toast
            anchors {
                horizontalCenter: parent.horizontalCenter
                bottom: parent.bottom
                bottomMargin: win.compact ? Theme.capsuleHeight + Theme.gap * 2 : Theme.gap * 2
            }
        }

        WindowFrame {
            id: frame
            anchors.fill: parent
            z: 8
        }

        Item {
            id: pointerLayer
            anchors.fill: parent
            z: 11
            readonly property Item popups: Overlay.overlay

            PointHandler {
                onActiveChanged: {
                    Nav.pointed(active)
                    const focused = win.activeFocusItem
                    if (!active || !(focused instanceof TextInput)) return
                    const x = point.scenePosition.x, y = point.scenePosition.y
                    let owner = null
                    for (let p = focused.parent; p && !owner; p = p.parent) if (typeof p.holds === "function") owner = p
                    const kept = owner ? owner.holds(x, y) : focused.contains(focused.mapFromItem(null, x, y))
                    if (!kept) focused.focus = false
                }
            }
        }

        PointHandler {
            parent: pointerLayer.popups
            onActiveChanged: Nav.pointed(active)
        }

        Image {
            id: before
            anchors.fill: parent
            z: 10
            opacity: 0
            visible: opacity > 0
            cache: false

            Ease {
                id: fade
                target: before
                property: "opacity"
                from: 1
                to: 0
                duration: Theme.restyleFade
                onStopped: before.source = ""
            }
        }
    }

    CoverCache {
        urls: {
            const out = Api.current ? [Api.current.thumb] : []
            for (let i = 0; i <= Theme.coversAhead; i++) if (Api.queue[Api.cursor + i]) out.push(Api.queue[Api.cursor + i].thumb)
            return out
        }
        shapes: {
            const square = px => ({ px: px, size: Qt.size(px, px), crop: true })
            const out = [{ px: Theme.tonePx, size: undefined, crop: false }, square(win.compact ? Theme.coverCapsulePx : Theme.coverColumnPx)]
            if (Nav.nowPlaying) out.push(square(Theme.coverNowPx))
            if (Nav.nowPlaying || Theme.glassLevel === "cover") out.push({ px: Theme.ambientPx, size: Qt.size(Theme.ambientDecode, 0), crop: true })
            return out
        }
    }

    CoverTone {
        source: Api.current ? Theme.art(Api.current.thumb, Theme.tonePx) : ""
    }

    WallpaperTone { id: wallpaper }

    BugReport { id: bugReport; stage: stage; window: win }

    Loader {
        active: !!win.argv.smoke || !!win.argv.nudgetest
        source: "dev/Smoke.qml"
        onLoaded: {
            if (win.argv.smoke) item.load(win.argv.smoke.split(","))
            if (win.argv.nudgetest) item.nudge()
        }
    }

    Loader {
        active: !!win.argv.scenario
        source: "dev/tests/Runner.qml"
        onLoaded: item.start(win.argv.scenario)
    }

    function loadSettings() {
        Api.call("getSetting", { key: "language" }, v => { if (v) Qt.uiLanguage = v })
        Api.call("getSetting", { key: "accent" }, v => { if (v) Theme.accentName = v })
        Api.call("getSetting", { key: "glass" }, v => { if (v !== null) Theme.glass = v })
        Api.call("getSetting", { key: "backgroundDim" }, v => { if (typeof v === "number") Theme.backgroundDim = Math.max(0, Math.min(1, v)) })
        Api.call("getSetting", { key: "navRight" }, v => { if (v !== null) Theme.navRight = v })
        Api.call("getSetting", { key: "sidebarCollapsed" }, v => {
            if (v === null) return
            Theme.sidebarCollapsed = v
            sidebar.settle()
        })
        Api.call("getSetting", { key: "closeToBackground" }, v => { win.closeToTray = !!v })
    }

    Component.onCompleted: {
        Core.windowBar(Theme.ink.toString(), Theme.frost.toString())
        loadSettings()
        if (argv.luma) Theme.wallpaperLuma = Number(argv.luma)
        else wallpaper.reload()
        if (argv.lang) Qt.uiLanguage = argv.lang
        if (argv.motion) tune(argv.motion)
        if (argv.slow) Motion.timeScale = Number(argv.slow)
        if (argv.view) Nav.go(argv.view, argv.params ? JSON.parse(argv.params) : {})
        if (argv.overlay) argv.overlay.split(",").forEach(o => { const [k, v] = o.split(":"); Nav[k] = v === undefined ? true : v })
        if (argv.play) Api.call("search", { q: argv.play, filter: "songs" }, r => { if (r && r.tracks.length) Api.radio(r.tracks[0]) })
        if (argv.shot) shotTimer.start()
    }

    function tune(json) {
        const asked = Api.parse(json, null)
        if (!asked || typeof asked !== "object") return console.warn("--motion: not a JSON object: " + json)
        for (const key of Object.keys(asked)) {
            const was = Motion[key]
            const value = asked[key]
            if (was === undefined || typeof was === "function") console.warn("--motion: Motion has no property " + key)
            else if (typeof value !== typeof was || (typeof value === "number" && !isFinite(value))) console.warn("--motion: Motion." + key + " takes a " + typeof was + ", not " + JSON.stringify(value))
            else Motion[key] = value
        }
    }

    Timer {
        id: shotTimer
        interval: Number(win.argv.delay || 4000)
        onTriggered: (win.argv.popups ? win.contentItem.parent : stage).grabToImage(r => { r.saveToFile(win.argv.shot); Qt.quit() })
    }

    readonly property bool typing: activeFocusItem instanceof TextInput
    readonly property real seekStep: 5

    readonly property var shortcuts: [
        { group: qsTr("Playback"), keys: ["Space"], label: qsTr("Play or pause"), global: true, when: () => !win.typing, run: () => Api.toggle() },
        { group: qsTr("Playback"), keys: ["Ctrl+Right"], label: qsTr("Next song"), run: () => Api.next() },
        { group: qsTr("Playback"), keys: ["Ctrl+Left"], label: qsTr("Previous song"), run: () => Api.prev() },
        { group: qsTr("Playback"), keys: ["Shift+Right"], label: qsTr("Forward 5 seconds"), when: () => !win.typing, run: () => Api.seek(Math.min(Api.duration, Api.position + win.seekStep)) },
        { group: qsTr("Playback"), keys: ["Shift+Left"], label: qsTr("Back 5 seconds"), when: () => !win.typing, run: () => Api.seek(Math.max(0, Api.position - win.seekStep)) },
        { group: qsTr("Playback"), keys: ["Ctrl+Up"], label: qsTr("Volume up"), run: () => Api.stepVolume(Api.volumeStep) },
        { group: qsTr("Playback"), keys: ["Ctrl+Down"], label: qsTr("Volume down"), run: () => Api.stepVolume(-Api.volumeStep) },
        { group: qsTr("Playback"), keys: ["Ctrl+M"], label: qsTr("Mute or unmute"), run: () => Api.toggleMute() },
        { group: qsTr("Playback"), keys: ["Ctrl+S"], label: qsTr("Shuffle on or off"), run: () => Api.toggleShuffle() },
        { group: qsTr("Playback"), keys: ["Ctrl+R"], label: qsTr("Repeat: off, all, one"), run: () => Api.cycleRepeat() },
        { group: qsTr("Playback"), keys: ["Alt+Shift+B"], label: qsTr("Like the playing song"), when: () => !!Api.current, run: () => Api.like(Api.current) },
        { group: qsTr("Navigation"), keys: ["Ctrl+K"], label: qsTr("Search"), run: () => sidebar.focusSearch() },
        { group: qsTr("Navigation"), keys: ["Ctrl+F"], label: qsTr("Find in this list or page"), run: () => Nav.finder ? Nav.finder.focusField() : sidebar.focusSearch() },
        { group: qsTr("Navigation"), keys: ["Alt+Left"], label: qsTr("Back"), run: () => Nav.back() },
        { group: qsTr("Navigation"), keys: ["Alt+Right"], label: qsTr("Forward"), run: () => Nav.forward() },
        { group: qsTr("Navigation"), keys: ["Ctrl+L"], label: qsTr("Lyrics"), run: () => { Nav.tab = "lyrics"; Nav.nowPlaying = Nav.nowPlaying || win.compact } },
        { group: qsTr("Navigation"), keys: ["Ctrl+N"], label: qsTr("New playlist"), run: () => sidebar.newPlaylist() },
        { group: qsTr("Navigation"), keys: ["Ctrl+,"], label: qsTr("Settings"), run: () => Nav.go("settings") },
        { group: qsTr("Navigation"), keys: ["Escape"], label: qsTr("Close Now Playing or the audio panel"), when: () => (Nav.audio || Nav.nowPlaying) && !Nudges.typing && !Nav.keys && !Nav.picking, run: () => { if (Nav.audio) Nav.audio = false; else if (Nav.nowPlaying) Nav.nowPlaying = false } },
        { group: qsTr("Song lists"), keys: ["Up", "Down"], label: qsTr("Move between songs") },
        { group: qsTr("Song lists"), keys: ["Return"], label: qsTr("Play the song") },
        { group: qsTr("Song lists"), keys: ["Shift+Up", "Shift+Down"], label: qsTr("Select more songs") },
        { group: qsTr("Song lists"), keys: ["Ctrl+A"], label: qsTr("Select all") },
        { group: qsTr("Song lists"), keys: ["Menu", "Shift+F10"], label: qsTr("Song menu") },
        { group: qsTr("Song lists"), keys: ["Escape"], label: qsTr("Clear the selection") },
        { group: qsTr("Settings"), keys: ["Up", "Down"], label: qsTr("Move between sections") },
        { group: qsTr("Help"), keys: ["Ctrl+/", "F1"], label: qsTr("Keyboard shortcuts"), when: () => !Nav.keys, run: () => Nav.keys = true },
        { group: qsTr("Help"), keys: ["Ctrl+Shift+B"], label: qsTr("Report a bug"), global: true, run: () => bugReport.capture() },
        { group: qsTr("Application"), keys: ["Ctrl+Q"], label: qsTr("Quit Snellar"), global: true, run: () => win.quit() }
    ]

    readonly property var acting: shortcuts.filter(s => s.run)
    readonly property Instantiator keyMap: Instantiator {
        model: win.acting.length
        delegate: Shortcut {
            required property int index
            readonly property var entry: win.acting[index]
            sequences: entry.keys
            context: entry.global ? Qt.ApplicationShortcut : Qt.WindowShortcut
            enabled: entry.when ? entry.when() : true
            onActivated: entry.run()
        }
    }

    KeyHelp { shortcuts: win.shortcuts }
}
