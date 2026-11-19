pragma ComponentBehavior: Bound
import QtQuick
import Snellar
import "../lists.js" as Lists

Item {
    id: view
    property var params: ({})
    property string language: Api.setting("language") || ""
    property string query: ""
    property string quality: Api.setting("quality") || "auto"
    property string section: "general"
    property bool componentsOpen: false
    readonly property bool split: width >= Theme.settingsSplitMin
    readonly property real paneWidth: head.width - (split ? Theme.settingsNavWidth + Theme.spaceXl : 0)
    readonly property bool closeToBackgroundOn: (Window.window as Main).closeToTray
    readonly property bool trayAddable: closeToBackgroundOn && !Api.trayReady && Theme.desktopName === "gnome"

    readonly property var qualityDetails: ({
        auto: qsTr("Opus up to 160 kbps, smooth between songs"),
        high: qsTr("The highest bitrate YouTube offers, any codec"),
        dataSaver: qsTr("The smallest stream, about 50 kbps")
    })

    readonly property var sections: [
        { id: "general", title: qsTr("General"), icon: "settings" },
        { id: "appearance", title: qsTr("Appearance"), icon: "droplet" },
        { id: "playback", title: qsTr("Playback"), icon: "music" },
        { id: "audio", title: qsTr("Audio"), icon: "volume" },
        { id: "library", title: qsTr("Library"), icon: "library" },
        { id: "services", title: qsTr("Connections"), icon: "share" },
        { id: "dependencies", title: qsTr("Dependencies"), icon: "box" },
        { id: "about", title: qsTr("About"), icon: "info" }
    ]
    readonly property var sectionIds: sections.map(s => s.id)

    readonly property var accentNames: ({
        glass: qsTr("Mint", "accent color"),
        lagoon: qsTr("Lagoon", "accent color"),
        teal: qsTr("Teal", "accent color"),
        lilac: qsTr("Lilac", "accent color"),
        coral: qsTr("Coral", "accent color"),
        cover: qsTr("Cover", "accent color")
    })

    readonly property var glassNames: ({
        auto: qsTr("Auto", "window background"),
        glass: qsTr("Glass", "window background"),
        cover: qsTr("Cover", "window background"),
        solid: qsTr("Solid", "window background")
    })
    readonly property var glassOptions: ["auto", "glass", "cover", "solid"].map(key => ({ key: key, label: glassNames[key] }))
    readonly property var sideOptions: [{ key: "left", label: qsTr("Left") }, { key: "right", label: qsTr("Right") }]

    readonly property var labels: ({
        language: { title: qsTr("Language"), control: "language" },
        accent: { title: qsTr("Accent color"), control: "accent", below: true, words: Object.values(accentNames).join(" ") },
        glass: { title: qsTr("Window background"), detail: qsTr("Auto is %1 on this desktop").arg(glassNames[Theme.autoGlass]), control: "glass" },
        backgroundDim: { title: qsTr("Dim the background"), control: "dim", hidden: !Theme.dimmable },
        navRight: { title: qsTr("Sidebar"), detail: qsTr("Flips the whole layout"), control: "side" },
        reduceMotion: { title: qsTr("Reduce motion"), detail: qsTr("The Home sky stays still and the sidebar folds instantly. Turns on by itself when your system asks for less motion") },
        autoplay: { title: qsTr("Autoplay"), detail: qsTr("Keep playing similar songs when the queue ends") },
        recsLive: { title: qsTr("Live versions"), detail: qsTr("In recommendations. When off, the original plays instead if Snellar finds it") },
        recsCovers: { title: qsTr("Covers"), detail: qsTr("In recommendations. When off, the original plays instead if Snellar finds it") },
        recsRemixes: { title: qsTr("Remixes"), detail: qsTr("In recommendations. When off, the original plays instead if Snellar finds it") },
        recsSpeed: { title: qsTr("Sped-up and slowed versions"), detail: qsTr("In recommendations. When off, the original plays instead if Snellar finds it") },
        recsSpam: { title: qsTr("Hide AI and spam artists"), detail: qsTr("In recommendations: songs marked as made by AI, and artists with hundreds of releases and few fans. Artists you know are never hidden") },
        loudness: { title: qsTr("Even out loudness"), detail: qsTr("Loud songs are turned down to the level of the rest, never up") },
        audioDevice: { title: qsTr("Output device"), detail: qsTr("If this device is unplugged, sound switches to Automatic until it's back"), control: "device" },
        closeToBackground: { title: qsTr("Keep playing when the window closes") },
        quality: { title: qsTr("Audio quality"), control: "quality" },
        handoff: { title: qsTr("Continue from other players"), detail: qsTr("Offers to pick up what another player is playing. Song titles are sent to YouTube to find them") },
        discordRpc: { title: qsTr("Show in Discord"), detail: qsTr("Your Discord status shows the song you're listening to"), words: "Rich Presence" },
        downloadDir: { title: qsTr("Download folder"), detail: qsTr("Where downloaded songs are saved"), hint: "~/Music/Snellar", control: "folder" },
        lastfmKey: { title: qsTr("Last.fm API key"), detail: qsTr("Optional. Finds similar artists for your recommendations"), link: { text: qsTr("Get a free key on Last.fm"), url: "https://www.last.fm/api/account/create" } },
        proxy: { title: qsTr("Proxy"), detail: qsTr("For YouTube and all other connections. Leave empty to use the system proxy."), hint: "http://host:3128" },
        autoUpdate: { title: qsTr("Update automatically"), words: "version", control: "update" }
    })

    property string discord: "off"
    function checkDiscord() { Api.call("discord", {}, v => { if (view) view.discord = v || "off" }) }
    Timer {
        interval: Theme.offlineRetry
        repeat: true
        triggeredOnStart: true
        running: view.visible && (view.section === "services" || view.query.length > 0)
        onTriggered: view.checkDiscord()
    }
    Connections {
        target: Api
        function onSettingSaved(key) { if (key === "discordRpc") view.checkDiscord() }
    }

    readonly property var liveDetails: ({
        accent: Theme.accentName === "cover" ? qsTr("Follows the cover of the song playing now") : qsTr("Colors buttons, the seek bar and the playing song"),
        backgroundDim: Theme.glassLevel === "cover" ? qsTr("How much of the cover shows behind the window. Double-click to reset") : qsTr("How dark the window is. Double-click to reset"),
        closeToBackground: trayAddable ? qsTr("GNOME has no tray, so closing the window quits Snellar. You can add one in Dependencies.") : Api.trayReady || !closeToBackgroundOn ? qsTr("Snellar stays in the tray; quit from its menu") : qsTr("No tray on this desktop, so closing the window quits Snellar"),
        quality: qsTr("%1. Applies from the next song").arg(qualityDetails[quality] || ""),
        discordRpc: discord === "waiting" ? qsTr("Discord isn't running. The song will show up once it opens.") : labels.discordRpc.detail
    })

    readonly property var blurSentence: {
        const info = Api.desktopInfo
        switch (info.desktop) {
        case "gnome": return info.blur ? qsTr("Glass works through Blur my Shell. Auto uses Glass.") : qsTr("GNOME doesn't blur windows by itself, so Auto uses Cover.")
        case "hyprland": return info.blur ? qsTr("Hyprland blurs this window. Auto uses Glass.") : qsTr("Blur is off in Hyprland, so Auto uses Cover.")
        case "swayfx": return info.blur ? qsTr("SwayFX blurs this window. Auto uses Glass.") : qsTr("Blur is off in SwayFX, so Auto uses Cover.")
        case "niri": return info.blur ? qsTr("Your niri config turns blur on. Auto stays on Cover: choose Glass in Appearance to use it.") : qsTr("niri doesn't blur this window yet, so Auto uses Cover.")
        case "kde": return qsTr("KDE Plasma blurs a window only when the app asks, and Snellar doesn't ask yet. Auto uses Cover.")
        case "cosmic": return qsTr("COSMIC blurs a window only when the app asks, and Snellar doesn't ask yet. Auto uses Cover.")
        case "sway": return qsTr("Sway can't blur windows, SwayFX can. Auto uses Cover.")
        default: return qsTr("Snellar can't tell whether this desktop blurs windows. Auto uses Cover.")
        }
    }

    readonly property var lineDetails: ({
        hyprland: qsTr("Add this to your Hyprland config. Snellar never edits your files."),
        swayfx: qsTr("Add this to your main SwayFX config, not an included file: only the main one can be read. Snellar never edits your files."),
        niri: qsTr("Add this to your niri config. Snellar never edits your files.")
    })

    readonly property var dependencyRows: {
        const info = Api.desktopInfo
        const better = qsTr("Can be improved")
        const here = qsTr("Dependencies")
        const rows = []
        if (info.desktop === "gnome" && !info.blur) rows.push({
            block: better,
            control: "glassOffer",
            below: true,
            title: qsTr("Glass through Blur my Shell"),
            detail: qsTr("Makes the window see-through. GNOME opens its own dialog to install the Blur my Shell extension, which also blurs the top bar and the overview. Snellar changes these extension settings: app blur on, Snellar on its list, opacity 255, dynamic opacity off, a Snellar pipeline for apps, radius 12 on all corners. Every app on its list gets this look. Undo puts them back and removes what Snellar installed.")
        })
        if (info.desktop === "gnome" && info.blur && info.gnome && !info.gnome.look) rows.push({
            block: better,
            control: "glassLook",
            below: true,
            title: qsTr("Lighter glass"),
            detail: qsTr("Blur my Shell darkens the window by default. This gives its apps a Snellar pipeline with softer blur, no darkening and radius 12 on all corners. Every app on its list gets this look. Undo puts it back.")
        })
        if (trayAddable) rows.push({
            block: better,
            control: "addTray",
            title: qsTr("Tray"),
            detail: qsTr("GNOME has no tray, so closing the window quits Snellar. GNOME opens its own dialog to install AppIndicator support.")
        })
        if (info.line) rows.push({
            block: better,
            control: "copyLine",
            below: true,
            title: qsTr("Window blur"),
            detail: info.line.startsWith("decoration") ? qsTr("Add this to your Hyprland config. This form, for Hyprland before 0.57, has not been tested. Snellar never edits your files.") : lineDetails[info.desktop] || ""
        })
        rows.push({ block: here, control: info.gnome && info.gnome.undo ? "undoGlass" : "", title: qsTr("Window blur"), detail: blurSentence, words: "glass blur" })
        rows.push({
            block: here,
            control: "components",
            below: true,
            title: qsTr("Components"),
            detail: qsTr("What Snellar runs on. Useful for bug reports."),
            words: "mpv yt-dlp ffmpeg qt portal"
        })
        return rows
    }

    readonly property string updateLine: {
        const u = updates.data
        if (!u) return ""
        const version = u.version
        const checked = u.checked ? qsTr("Last checked %1").arg(Theme.since(u.checked).toLowerCase()) : qsTr("Not checked yet")
        switch (u.phase) {
        case "development": return qsTr("Version %1. Development builds don't update themselves").arg(version)
        case "elsewhere": return qsTr("Version %1, running from %2. Only a copy installed by the install script updates itself").arg(version).arg(u.place)
        case "unsigned": return qsTr("Version %1. This build has no update key, so it doesn't update itself").arg(version)
        case "off": return qsTr("Version %1. Not checking for updates").arg(version)
        case "checking": return qsTr("Version %1. Checking for updates…").arg(version)
        case "downloading": return qsTr("Version %1. Downloading %2: %3%").arg(version).arg(u.latest).arg(u.percent)
        case "current": return qsTr("Version %1 is up to date. %2").arg(version).arg(checked)
        case "ready": return qsTr("Version %1 is ready. It starts next time you open Snellar").arg(u.latest)
        case "failed": return qsTr("Version %1. Couldn't update: %2 Snellar will try again later").arg(version).arg(sentence(Errors.text(u.code)))
        default: return qsTr("Version %1. %2").arg(version).arg(checked)
        }
    }

    readonly property string updatePhase: updates.data ? updates.data.phase : "development"
    readonly property bool updateManaged: updatePhase !== "development" && updatePhase !== "elsewhere" && updatePhase !== "unsigned"
    readonly property bool updateBusy: updatePhase === "checking" || updatePhase === "downloading"

    function sentence(text) { return /[.!?]$/.test(text) ? text : text + "." }

    Fetch { id: updates; method: "updateStatus" }

    Connections {
        target: Api
        function onUpdateChanged() { updates.reload(true) }
        function onSettingSaved(key) { if (key === "autoUpdate") updates.reload(true) }
    }

    readonly property var extras: [
        { section: "library", control: "clearHistory", title: qsTr("Clear history"), detail: qsTr("Forget everything you've played") },
        { section: "about", control: "version", title: "Snellar", detail: qsTr("Music player"), first: true },
        { section: "about", control: "keys", title: qsTr("Keyboard shortcuts"), detail: qsTr("Also opens with Ctrl+/") }
    ]

    function rowsOf(section) {
        if (section.id === "dependencies") return dependencyRows
        const fromRegistry = Api.registry.filter(s => s.section === section.id).map(s => Object.assign(
            { key: s.key, control: s.kind === "bool" ? "switch" : "field", title: s.key, detail: "" }, labels[s.key])).filter(row => !row.hidden)
        const added = extras.filter(e => e.section === section.id)
        return added.filter(e => e.first).concat(fromRegistry, added.filter(e => !e.first))
    }

    function detailOf(row) { return liveDetails[row.key] ?? row.detail }

    readonly property var groups: {
        const words = Lists.fold(query).split(/\s+/).filter(w => w.length)
        const fits = (row, section) => {
            const hay = Lists.fold([row.title, detailOf(row), section.title, row.key || "", row.words || ""].join(" "))
            return words.every(w => hay.indexOf(w) >= 0)
        }
        const searched = words.length ? sections : sections.filter(s => s.id === view.section)
        const out = []
        for (const section of searched) {
            const rows = rowsOf(section).filter(row => fits(row, section))
            for (const block of Array.from(new Set(rows.map(row => row.block || section.title))))
                out.push({ section: { title: block }, rows: rows.filter(row => (row.block || section.title) === block) })
        }
        return out
    }

    function stepSection(by) {
        const at = sectionIds.indexOf(section) + by
        if (at >= 0 && at < sectionIds.length) pickSection(sectionIds[at])
    }

    function pickSection(id) {
        query = ""
        section = id
    }

    onParamsChanged: if (params.section) section = params.section
    onSectionChanged: {
        Nav.settingsSection = section
        pane.contentY = 0
    }

    function setLanguage(code) {
        language = code
        Theme.restyle(() => Qt.uiLanguage = code || Qt.locale().name)
        Api.call("setSetting", { key: "language", value: code })
    }

    function setAppearance(key, value) {
        Theme.restyle(() => {
            if (key === "accent") Theme.accentName = value
            else Theme[key] = value
        })
        Api.call("setSetting", { key: key, value: value })
    }

    function setDim(value) {
        if (value === Theme.backgroundDim) return
        Theme.backgroundDim = value
        Api.call("setSetting", { key: "backgroundDim", value: value })
    }

    Component.onCompleted: {
        nav.forceActiveFocus()
        if (Nav.settingsSection) section = Nav.settingsSection
        Api.call("getSetting", { key: "language" }, v => { if (view) view.language = v || "" })
        Api.call("getSetting", { key: "quality" }, v => { if (view && v) view.quality = v })
        Nav.finder = find
    }
    Component.onDestruction: if (Nav.finder === find) Nav.finder = null

    Item {
        id: head
        x: Theme.space2xl
        y: Theme.navBar
        width: Math.min(view.width - 2 * Theme.space2xl, (view.split ? Theme.settingsNavWidth + Theme.spaceXl : 0) + Theme.settingsWidth)
        height: Theme.controlLg

        PageTitle { id: title; text: qsTr("Settings") }
        FindField {
            id: find
            anchors { right: parent.right; verticalCenter: title.verticalCenter }
            hint: qsTr("Find a setting")
            query: view.query
            onEdited: text => view.query = text
            onDismissed: nav.forceActiveFocus()
        }
    }

    Flow {
        id: nav
        x: Theme.space2xl
        y: head.y + head.height + Theme.spaceLg
        width: view.split ? Theme.settingsNavWidth : head.width
        spacing: Theme.spaceXs
        activeFocusOnTab: true
        Keys.onUpPressed: view.stepSection(-1)
        Keys.onDownPressed: view.stepSection(1)
        Keys.onLeftPressed: event => { event.accepted = !view.split; if (!view.split) view.stepSection(-1) }
        Keys.onRightPressed: event => { event.accepted = !view.split; if (!view.split) view.stepSection(1) }

        Repeater {
            model: view.sections
            delegate: SidebarRow {
                required property var modelData
                activeFocusOnTab: false
                width: view.split ? nav.width : implicitWidth
                icon: modelData.icon
                text: modelData.title
                active: view.query.length === 0 && view.section === modelData.id
                onClicked: {
                    view.pickSection(modelData.id)
                    nav.forceActiveFocus()
                }
            }
        }
    }

    PageScroll {
        id: pane
        x: view.split ? nav.x + nav.width + Theme.spaceXl : Theme.space2xl
        y: view.split ? nav.y : nav.y + nav.height + Theme.spaceLg
        width: view.width - x
        height: view.height - y
        padding: 0
        headroom: 0

        EmptyState {
            visible: view.groups.length === 0
            width: view.paneWidth
            icon: "search"
            text: qsTr("No setting matches “%1”").arg(view.query.trim())
        }

        Repeater {
            model: view.groups

            delegate: Column {
                id: group
                required property var modelData
                width: view.paneWidth
                spacing: Theme.spaceSm

                SectionTitle { text: group.modelData.section.title }

                Rectangle {
                    width: parent.width
                    height: card.height + 2 * Theme.spaceSm
                    radius: Theme.rCard
                    color: Theme.fillRaised

                    Column {
                        id: card
                        x: Theme.spaceXl
                        y: Theme.spaceSm
                        width: parent.width - 2 * Theme.spaceXl

                        Repeater {
                            model: group.modelData.rows

                            delegate: SettingRow {
                                id: row
                                required property var modelData
                                required property int index
                                width: card.width
                                title: modelData.title
                                detail: modelData.key === "autoUpdate" ? view.updateLine : view.detailOf(modelData)
                                below: !!modelData.below
                                link: modelData.link || null
                                divided: index > 0

                                Loader {
                                    sourceComponent: ({
                                        language: languageControl, accent: accentControl, glass: glassControl, dim: dimControl, side: sideControl,
                                        "switch": switchControl, field: fieldControl, folder: folderControl, quality: qualityControl, device: deviceControl,
                                        clearHistory: clearControl, version: versionControl, update: updateControl, keys: keysControl,
                                        glassOffer: glassOfferControl, glassLook: glassLookControl, addTray: addTrayControl, copyLine: copyLineControl, undoGlass: undoGlassControl, components: componentsControl
                                    })[row.modelData.control] || null
                                }

                                Component {
                                    id: languageControl
                                    IosSegment {
                                        current: view.language
                                        options: [{ key: "", label: qsTr("System") }, { key: "en", label: "English" }, { key: "ru", label: "Русский" }, { key: "uk", label: "Українська" }]
                                        onPicked: key => view.setLanguage(key)
                                    }
                                }
                                Component {
                                    id: accentControl
                                    Swatches {
                                        width: row.width
                                        names: view.accentNames
                                        onPicked: name => view.setAppearance("accent", name)
                                    }
                                }
                                Component {
                                    id: glassControl
                                    IosSegment {
                                        width: fit
                                        current: Theme.glass
                                        options: view.glassOptions
                                        peers: view.sideOptions
                                        onPicked: key => view.setAppearance("glass", key)
                                    }
                                }
                                Component {
                                    id: dimControl
                                    FxSlider {
                                        width: Theme.segmentNarrow
                                        value: Theme.backgroundDim
                                        neutral: Api.defaults.backgroundDim
                                        step: Theme.dimStep
                                        onMoved: v => view.setDim(v)
                                    }
                                }
                                Component {
                                    id: sideControl
                                    IosSegment {
                                        width: fit
                                        current: Theme.navRight ? "right" : "left"
                                        options: view.sideOptions
                                        peers: view.glassOptions
                                        onPicked: key => view.setAppearance("navRight", key === "right")
                                    }
                                }
                                Component {
                                    id: switchControl
                                    SettingToggle { key: row.modelData.key }
                                }
                                Component {
                                    id: updateControl
                                    Row {
                                        spacing: Theme.spaceMd
                                        PillButton {
                                            visible: view.updateManaged
                                            height: Theme.controlSm
                                            enabled: !view.updateBusy
                                            primary: view.updatePhase === "ready"
                                            text: view.updatePhase === "ready" ? qsTr("Restart") : qsTr("Check now")
                                            onClicked: {
                                                if (view.updatePhase === "ready") Api.restartForUpdate()
                                                else Api.checkUpdate()
                                            }
                                        }
                                        SettingToggle { anchors.verticalCenter: parent.verticalCenter; key: row.modelData.key }
                                    }
                                }
                                Component {
                                    id: glassOfferControl
                                    PillButton {
                                        primary: true
                                        enabled: !Api.glassBusy
                                        text: Api.glassBusy ? qsTr("Waiting for GNOME") : qsTr("Set up glass")
                                        onClicked: Api.setupGlass()
                                    }
                                }
                                Component {
                                    id: glassLookControl
                                    PillButton {
                                        primary: true
                                        enabled: !Api.glassBusy
                                        text: qsTr("Update glass")
                                        onClicked: Api.setupGlass()
                                    }
                                }
                                Component {
                                    id: addTrayControl
                                    PillButton {
                                        text: qsTr("Add tray support")
                                        onClicked: Api.addTray()
                                    }
                                }
                                Component {
                                    id: undoGlassControl
                                    PillButton {
                                        enabled: !Api.glassBusy
                                        text: qsTr("Undo")
                                        onClicked: Api.undoGlass()
                                    }
                                }
                                Component {
                                    id: copyLineControl
                                    Column {
                                        width: row.width
                                        spacing: Theme.spaceSm
                                        Text {
                                            width: parent.width
                                            text: Api.desktopInfo.line || ""
                                            color: Theme.text
                                            font.family: Theme.fontMono
                                            font.pixelSize: Theme.fsCaption
                                            wrapMode: Text.WrapAnywhere
                                        }
                                        PillButton {
                                            text: qsTr("Copy")
                                            onClicked: {
                                                Api.copyText(Api.desktopInfo.line)
                                                Api.toast(qsTr("Copied"))
                                            }
                                        }
                                    }
                                }
                                Component {
                                    id: componentsControl
                                    ComponentList {
                                        width: row.width
                                        open: view.componentsOpen
                                        onToggled: view.componentsOpen = !view.componentsOpen
                                    }
                                }
                                Component {
                                    id: fieldControl
                                    SettingField { key: row.modelData.key; hint: row.modelData.hint || "" }
                                }
                                Component {
                                    id: folderControl
                                    Row {
                                        spacing: Theme.spaceSm

                                        SettingField { id: folderField; key: row.modelData.key; hint: row.modelData.hint || "" }
                                        PillButton {
                                            height: Theme.controlSm
                                            text: qsTr("Browse")
                                            icon: "folder"
                                            onClicked: folderPicker.open()
                                        }
                                        FilePicker {
                                            id: folderPicker
                                            folder: true
                                            title: qsTr("Choose a download folder")
                                            start: /^(\/|~\/|~$)/.test(folderField.text.trim()) ? folderField.text.trim() : ""
                                            onPicked: path => folderField.assign(path)
                                        }
                                    }
                                }
                                Component {
                                    id: qualityControl
                                    IosSegment {
                                        current: view.quality
                                        options: [{ key: "auto", label: qsTr("Auto", "audio quality") }, { key: "high", label: qsTr("High", "audio quality") }, { key: "dataSaver", label: qsTr("Data saver", "audio quality") }]
                                        onPicked: key => {
                                            view.quality = key
                                            Api.call("setSetting", { key: "quality", value: key })
                                        }
                                    }
                                }
                                Component {
                                    id: deviceControl
                                    DeviceChoice { textLimit: row.width - 2 * Theme.pillPadding - Theme.iconSm - Theme.spaceSm }
                                }
                                Component {
                                    id: versionControl
                                    Text { text: Core.version(); color: Theme.text2; font.pixelSize: Theme.fsBody }
                                }
                                Component {
                                    id: keysControl
                                    PillButton {
                                        text: qsTr("Show")
                                        icon: "keyboard"
                                        onClicked: Nav.keys = true
                                    }
                                }
                                Component {
                                    id: clearControl
                                    PillButton {
                                        id: clearButton
                                        text: qsTr("Clear")
                                        icon: "trash"
                                        onClicked: clearHistory.openFrom(clearButton)
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Item { width: 1; height: Theme.space2xl }
    }

    Confirm {
        id: clearHistory
        title: qsTr("Clear your listening history?")
        text: qsTr("Recommendations will start over from your favorites. You can't undo this.")
        action: qsTr("Clear history")
        onAccepted: Api.call("clearHistory", {}, () => { Api.libraryChanged(); Api.toast(qsTr("History cleared")) })
    }
}
