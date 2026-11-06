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
        else if (action === "quit") {
            quitting = true
            Qt.quit()
        }
    }

    property bool quitting: false

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
        layer.effect: ShaderEffect {
            property real wPx: width
            property real hPx: height
            property real radiusPx: Theme.windowRadius
            fragmentShader: "../shaders/round.frag.qsb"
        }

        TapHandler {
            acceptedButtons: Qt.BackButton | Qt.ForwardButton
            onTapped: (point, button) => button === Qt.BackButton ? Nav.back() : Nav.forward()
        }

        PointHandler {
            onActiveChanged: {
                const focused = win.activeFocusItem
                if (!active || !(focused instanceof TextInput)) return
                const at = focused.mapFromItem(null, point.scenePosition.x, point.scenePosition.y)
                if (!focused.contains(at)) focused.focus = false
            }
        }

        Rectangle {
            anchors.fill: parent
            visible: !!win.argv.shot
            gradient: Gradient {
                GradientStop { position: 0; color: win.backdropTop }
                GradientStop { position: 1; color: win.backdropBottom }
            }
        }

        Item {
            anchors.fill: parent
            visible: !(nowLoader.item && (nowLoader.item as NowPlaying).covering)

            Rectangle { anchors.fill: parent; visible: Theme.glassLevel === "cover"; color: Theme.shade }
            Ambient {
                anchors.fill: parent
                visible: Theme.glassLevel === "cover"
                source: visible && Api.current ? Theme.art(Api.current.thumb, 160) : ""
            }
            Rectangle { anchors.fill: parent; color: Theme.window }

            WindowDrag {
                anchors.fill: parent
                onDoubleTapped: frame.toggleMaximized()
            }

            Sidebar {
                id: sidebar
                anchors { left: parent.left; top: parent.top; bottom: parent.bottom }
                width: Theme.sidebarWidth
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false
            }

            Item {
                id: content
                anchors {
                    left: sidebar.right
                    right: win.compact ? parent.right : column.left
                    top: parent.top
                    bottom: parent.bottom
                }
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false

                ViewHost {
                    Behavior on anchors.topMargin { NumberAnimation { duration: Theme.fast; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard } }
                    anchors { fill: parent; topMargin: Nav.canBack || Nav.canForward ? Theme.toolbarHeight : 0; bottomMargin: win.compact ? Theme.capsuleHeight + 2 * Theme.gap : 0 }
                }

                WindowDrag {
                    anchors { left: parent.left; right: parent.right; top: parent.top }
                    height: Theme.spaceXl
                    visible: Theme.frameless
                    onDoubleTapped: frame.toggleMaximized()
                }

                Row {
                    x: Theme.space2xl
                    y: Theme.spaceMd
                    visible: Nav.canBack || Nav.canForward
                    spacing: Theme.spaceXs

                    IconButton {
                        icon: "back"
                        tip: qsTr("Back")
                        enabled: Nav.canBack
                        opacity: enabled ? 1 : Theme.disabledOpacity
                        onClicked: Nav.back()
                    }
                    IconButton {
                        icon: "forward"
                        tip: qsTr("Forward")
                        enabled: Nav.canForward
                        opacity: enabled ? 1 : Theme.disabledOpacity
                        onClicked: Nav.forward()
                    }
                }
            }

            NowColumn {
                id: column
                anchors {
                    right: parent.right
                    top: parent.top
                    bottom: parent.bottom
                    margins: Theme.gap
                    topMargin: Theme.frameless ? Theme.windowBarHeight : Theme.gap
                }
                width: Theme.inspectorWidth
                visible: !win.compact
                LayoutMirroring.enabled: Theme.navRight
                LayoutMirroring.childrenInherit: false
            }

            PlayerCapsule {
                id: capsule
                anchors {
                    horizontalCenter: content.horizontalCenter
                    bottom: parent.bottom
                    bottomMargin: Theme.gap
                }
                width: content.width - 2 * Theme.space2xl
                visible: win.compact
            }
        }

        Loader {
            id: nowLoader
            anchors.fill: parent
            active: Nav.nowPlaying || item !== null
            source: "parts/NowPlaying.qml"
        }

        Loader {
            id: audioLoader
            readonly property NowPlaying now: Nav.nowPlaying ? nowLoader.item as NowPlaying : null
            anchors.fill: parent
            active: Nav.audio || item !== null
            source: "parts/AudioPanel.qml"
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

        Rectangle {
            id: unsaved
            readonly property bool active: !!Api.libraryProblem && !Api.libraryProblem.saving
            visible: active
            z: 5
            anchors { horizontalCenter: parent.horizontalCenter; top: parent.top; topMargin: Theme.gap }
            width: Math.min(parent.width - 2 * Theme.gap, unsavedRow.implicitWidth + 2 * Theme.pillPadding)
            height: Theme.controlLg
            radius: height / 2
            color: Theme.toastFill
            border.width: 1
            border.color: Theme.hairline

            Row {
                id: unsavedRow
                anchors.centerIn: parent
                spacing: Theme.spaceMd
                Icon { anchors.verticalCenter: parent.verticalCenter; name: "alert"; size: Theme.iconSm; color: Theme.red }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("Not saving: changes will be lost when Snellar closes")
                    color: Theme.text
                    font.pixelSize: Theme.fsBody
                }
                Link {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("Try again")
                    color: Theme.accent
                    font.pixelSize: Theme.fsBody
                    font.weight: Font.DemiBold
                    onClicked: Api.call("libraryRetry", {}, opened => { if (opened) Api.libraryChanged() })
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

        Image {
            id: before
            anchors.fill: parent
            z: 10
            opacity: 0
            visible: opacity > 0
            cache: false

            NumberAnimation {
                id: fade
                target: before
                property: "opacity"
                from: 1
                to: 0
                duration: Theme.restyleFade
                easing.type: Easing.BezierSpline
                easing.bezierCurve: Theme.curveStandard
                onStopped: before.source = ""
            }
        }
    }

    CoverTone {
        source: Api.current ? Theme.art(Api.current.thumb, 60) : ""
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
        Api.call("getSetting", { key: "navRight" }, v => { if (v !== null) Theme.navRight = v })
        Api.call("getSetting", { key: "closeToBackground" }, v => { win.closeToTray = !!v })
    }

    Component.onCompleted: {
        Core.windowBar(Theme.ink.toString(), Theme.frost.toString())
        loadSettings()
        if (argv.luma) Theme.wallpaperLuma = Number(argv.luma)
        else wallpaper.reload()
        if (argv.lang) Qt.uiLanguage = argv.lang
        if (argv.view) Nav.go(argv.view, argv.params ? JSON.parse(argv.params) : {})
        if (argv.overlay) argv.overlay.split(",").forEach(o => { const [k, v] = o.split(":"); Nav[k] = v === undefined ? true : v })
        if (argv.play) Api.call("search", { q: argv.play, filter: "songs" }, r => { if (r && r.tracks.length) Api.radio(r.tracks[0]) })
        if (argv.shot) shotTimer.start()
    }

    Timer {
        id: shotTimer
        interval: Number(win.argv.delay || 4000)
        onTriggered: stage.grabToImage(r => { r.saveToFile(win.argv.shot); Qt.quit() })
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
        { group: qsTr("Playback"), keys: ["Ctrl+S"], label: qsTr("Shuffle on or off"), run: () => Api.call("shuffle", { on: !Api.shuffle }) },
        { group: qsTr("Playback"), keys: ["Ctrl+R"], label: qsTr("Repeat: off, all, one"), run: () => Api.call("repeat", { mode: Api.repeat === "off" ? "all" : (Api.repeat === "all" ? "one" : "off") }) },
        { group: qsTr("Playback"), keys: ["Alt+Shift+B"], label: qsTr("Like the current song"), when: () => !!Api.current, run: () => Api.like(Api.current) },
        { group: qsTr("Navigation"), keys: ["Ctrl+K"], label: qsTr("Search"), run: () => sidebar.focusSearch() },
        { group: qsTr("Navigation"), keys: ["Ctrl+F"], label: qsTr("Find in this list or page"), run: () => Nav.finder ? Nav.finder.focusField() : sidebar.focusSearch() },
        { group: qsTr("Navigation"), keys: ["Alt+Left"], label: qsTr("Back"), run: () => Nav.back() },
        { group: qsTr("Navigation"), keys: ["Alt+Right"], label: qsTr("Forward"), run: () => Nav.forward() },
        { group: qsTr("Navigation"), keys: ["Ctrl+L"], label: qsTr("Lyrics"), run: () => { Nav.tab = "lyrics"; Nav.nowPlaying = win.compact } },
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
        { group: qsTr("Help"), keys: ["Ctrl+Shift+B"], label: qsTr("Report a bug"), global: true, run: () => bugReport.capture() }
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
