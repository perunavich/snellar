pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

SidePanel {
    id: bar
    lineRight: !Theme.navRight
    width: Theme.railWidth + (Theme.sidebarWidth - Theme.railWidth) * (1 - bar.fold)

    property bool narrow: false
    property bool peek: false
    property bool searching: false
    readonly property bool held: creating || Nav.renamingFolder !== null
    readonly property bool rail: !peek && !held && (Theme.sidebarCollapsed || narrow)
    readonly property real fold: folding.value
    readonly property bool moving: folding.moving
    readonly property real words: Motion.railWords(fold)

    SpringValue {
        id: folding
        response: Motion.railResponse
        zeta: Motion.railDamping
    }

    function settle() {
        folding.target = rail ? 1 : 0
        folding.snap(folding.target)
    }

    onRailChanged: {
        if (Motion.reduced) settle()
        else folding.target = rail ? 1 : 0
    }
    onNarrowChanged: if (!narrow) peek = false

    function choose(collapsed) {
        Theme.sidebarCollapsed = collapsed
        Api.call("setSetting", { key: "sidebarCollapsed", value: collapsed })
    }

    function toggle() {
        if (rail) {
            if (Theme.sidebarCollapsed) choose(false)
            peek = narrow
            return
        }
        peek = false
        creating = false
        if (Nav.renamingFolder !== null) finishRename()
        if (!narrow) choose(true)
    }

    function closeSearch() {
        searching = false
        search.leave()
    }

    readonly property bool windowActive: Window.active
    onWindowActiveChanged: if (!windowActive && searching) closeSearch()

    Connections {
        target: Nav
        function onArrived() { if (bar.searching) bar.closeSearch() }
    }

    function openSearch() {
        searching = true
        search.focusField()
    }

    property var playlists: []
    property var remote: []
    property var folders: []
    property string folderDraft: ""
    property Item renameField: null

    function finishRename() {
        const id = Nav.renamingFolder
        const name = folderDraft.trim()
        Nav.renamingFolder = null
        folderDraft = ""
        if (id !== null && name) Api.call("renameFolder", { id: id, name: name }, () => refresh())
    }

    function renameBlurred() {
        if (Nav.renamingFolder !== null && !(renameField && renameField.activeFocus)) finishRename()
    }
    property bool creating: false
    readonly property var pinned: Api.pins.map(p => p.kind === "playlist" ? { kind: p.kind, item: playlists.find(l => String(l.id) === p.id) } : p).filter(p => !!p.item)
    readonly property var entries: {
        const loose = playlists.filter(p => !Api.isPinned("playlist", p.id))
        const out = []
        for (const f of folders) {
            const inside = loose.filter(p => p.folder === f.id)
            out.push({ kind: "folder", item: Object.assign({ count: inside.length }, f) })
            if (f.open) inside.forEach(p => out.push({ kind: "playlist", item: p, nested: true }))
        }
        loose.filter(p => !folders.some(f => f.id === p.folder)).forEach(p => out.push({ kind: "playlist", item: p }))
        return out
    }
    property var counts: ({ favorites: 0, albums: 0, artists: 0 })

    function focusSearch() {
        if (rail) openSearch()
        else search.focusField()
    }

    function newPlaylist() {
        creating = true
        field.forceActiveFocus()
    }

    function refresh() {
        const keep = (key, v) => { if (JSON.stringify(v || []) !== JSON.stringify(bar[key])) bar[key] = v || [] }
        Api.call("playlists", {}, v => keep("playlists", v))
        Api.call("savedPlaylists", {}, v => keep("remote", v))
        Api.call("folders", {}, v => keep("folders", v))
        Api.call("favorites", {}, v => bar.counts = Object.assign({}, bar.counts, { favorites: v ? v.length : 0 }))
        Api.call("savedAlbums", {}, v => bar.counts = Object.assign({}, bar.counts, { albums: v ? v.length : 0 }))
        Api.call("followed", {}, v => bar.counts = Object.assign({}, bar.counts, { artists: v ? v.length : 0 }))
    }

    function create() {
        const name = field.text.trim()
        creating = false
        if (name) Api.call("createPlaylist", { name: name }, id => { Api.libraryChanged(); Nav.playlist(id) })
    }

    onCreatingChanged: if (!creating) field.text = ""

    Component.onCompleted: {
        settle()
        refresh()
    }
    Connections {
        target: Api
        function onLibraryChanged() { bar.refresh() }
        function onPlaylistTracksRemoved() { bar.refresh() }
    }

    Column {
        id: top
        z: 5
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.spaceMd }
        spacing: Theme.spaceXs

        Item {
            id: head
            width: parent.width
            height: Theme.sidebarRow
            z: 5

            Item {
                id: slot
                anchors { left: parent.left; right: toggleButton.left; rightMargin: Theme.spaceXs }
                height: parent.height
            }

            IconButton {
                id: toggleButton
                anchors.right: parent.right
                size: Theme.sidebarRow
                iconSize: Theme.iconSm
                icon: Theme.navRight ? "sidebarRight" : "sidebar"
                tip: bar.rail ? qsTr("Expand sidebar") : qsTr("Collapse sidebar")
                activeFocusOnTab: true
                backed: activeFocus
                Accessible.role: Accessible.Button
                Accessible.name: tip
                Keys.onReturnPressed: bar.toggle()
                Keys.onEnterPressed: bar.toggle()
                onClicked: bar.toggle()
            }
        }

        Item {
            id: searchSlot
            width: parent.width
            height: Theme.spaceSm + (Theme.sidebarRow - Theme.spaceSm) * bar.fold

            SidebarRow {
                id: searchRow
                width: parent.width
                fold: bar.fold
                visible: bar.fold >= 1
                bare: true
                icon: "search"
                text: qsTr("Search")
                active: Nav.view === "search"
                onClicked: bar.openSearch()
            }
        }

        SidebarRow { width: parent.width; fold: bar.fold; icon: "home"; text: qsTr("Home"); active: Nav.view === "discover"; onClicked: Nav.go("discover") }

        Item {
            width: parent.width
            height: Theme.controlMd + (Theme.spaceSm - Theme.controlMd) * bar.fold

            Heading {
                anchors { left: parent.left; leftMargin: Theme.spaceSm; bottom: parent.bottom; bottomMargin: (Theme.fieldButton - height) / 2 }
                visible: opacity > 0
                opacity: bar.words
                text: qsTr("Library")
            }
        }
        SidebarRow { width: parent.width; fold: bar.fold; icon: "heart"; text: qsTr("Favorites"); active: Nav.view === "favorites"; count: bar.counts.favorites; onClicked: Nav.go("favorites") }
        SidebarRow { width: parent.width; fold: bar.fold; icon: "download"; text: qsTr("Downloaded"); active: Nav.view === "downloaded" || Nav.view === "downloadedSongs"; count: Object.keys(Api.downloadedIds).length; onClicked: Nav.go("downloaded") }
        SidebarRow { width: parent.width; fold: bar.fold; icon: "disc"; text: qsTr("Albums"); active: Nav.view === "albums"; count: bar.counts.albums; onClicked: Nav.go("albums") }
        SidebarRow { width: parent.width; fold: bar.fold; icon: "user"; text: qsTr("Artists"); active: Nav.view === "artists"; count: bar.counts.artists; onClicked: Nav.go("artists") }
        SidebarRow { width: parent.width; fold: bar.fold; icon: "history"; text: qsTr("History"); active: Nav.view === "history"; onClicked: Nav.go("history") }

        Item {
            width: parent.width
            height: pinnedTitle.height + (Theme.spaceSm - pinnedTitle.height) * bar.fold
            visible: bar.pinned.length > 0

            Heading {
                id: pinnedTitle
                visible: opacity > 0
                opacity: bar.words
                text: qsTr("Pinned")
                topPadding: Theme.spaceLg
                leftPadding: Theme.spaceSm
            }
        }
        Repeater {
            model: bar.pinned
            delegate: ListRow {
                required property var modelData
                width: parent.width
                kind: modelData.kind
                entity: modelData.item
            }
        }

        Item {
            width: parent.width
            height: Theme.controlMd + (Theme.spaceSm + Theme.spaceXs + Theme.sidebarRow - Theme.controlMd) * bar.fold

            Item {
                id: playlistsTitle
                width: parent.width
                height: Theme.controlMd + (Theme.spaceSm - Theme.controlMd) * bar.fold
            }
            Heading {
                visible: opacity > 0
                opacity: bar.words
                anchors { left: parent.left; leftMargin: Theme.spaceSm; verticalCenter: addPlaylist.verticalCenter }
                text: qsTr("Playlists")
            }
            IconButton {
                id: addPlaylist
                x: parent.width - Theme.fieldButton - (Theme.sidebarRow - Theme.fieldButton) / 2 * bar.fold
                y: playlistsTitle.height - Theme.fieldButton + (Theme.spaceXs + (Theme.sidebarRow + Theme.fieldButton) / 2) * bar.fold
                visible: bar.fold < 1
                icon: bar.creating ? "close" : "plus"
                tip: bar.creating ? qsTr("Cancel") : qsTr("New playlist")
                size: Theme.fieldButton
                iconSize: Theme.iconSm + (Theme.iconMd - Theme.iconSm) * bar.fold
                onClicked: {
                    bar.creating = !bar.creating
                    if (bar.creating) field.forceActiveFocus()
                }
            }
            SidebarRow {
                id: newRow
                y: playlistsTitle.height + Theme.spaceXs
                width: parent.width
                fold: bar.fold
                visible: bar.fold >= 1
                bare: true
                icon: "plus"
                text: qsTr("New playlist")
                onClicked: bar.newPlaylist()
            }
        }

        TextField {
            id: field
            width: parent.width
            height: Theme.controlSm
            visible: bar.creating
            leftPadding: Theme.spaceMd
            placeholderText: qsTr("Playlist name")
            placeholderTextColor: Theme.text3
            color: Theme.text
            font.pixelSize: Theme.fsCallout
            background: Rectangle { radius: height / 2; color: Theme.fillRaised }
            onAccepted: bar.create()
            Keys.onEscapePressed: bar.creating = false
        }

        Row {
            id: createActions
            width: parent.width
            visible: bar.creating
            topPadding: Theme.spaceXs
            bottomPadding: Theme.spaceSm
            spacing: Theme.spaceSm

            PillButton {
                width: (createActions.width - createActions.spacing) / 2
                height: Theme.controlSm
                text: qsTr("Import")
                onClicked: {
                    bar.creating = false
                    Nav.go("import")
                }
            }
            PillButton {
                width: (createActions.width - createActions.spacing) / 2
                height: Theme.controlSm
                opacity: field.text.trim() ? 1 : Theme.disabledOpacity
                primary: true
                text: qsTr("Create")
                onClicked: {
                    if (field.text.trim()) bar.create()
                    else field.forceActiveFocus()
                }
            }
        }
    }

    ListView {
        id: lists
        anchors { left: parent.left; right: parent.right; top: top.bottom; bottom: settings.top; leftMargin: Theme.spaceMd; rightMargin: Theme.spaceMd }
        clip: true
        spacing: Theme.spaceXs
        model: bar.entries
        boundsBehavior: Flickable.StopAtBounds

        delegate: ListRow {
            required property var modelData
            width: ListView.view.width
            kind: modelData.kind
            entity: modelData.item
            indent: modelData.nested ? Theme.spaceLg : 0
        }
        footer: Column {
            width: lists.width
            topPadding: bar.remote.length ? Theme.spaceXs : 0
            spacing: Theme.spaceXs
            Repeater {
                model: bar.remote.filter(p => !Api.isPinned("remote", p.id))
                delegate: ListRow { required property var modelData; width: parent.width; kind: "remote"; entity: modelData }
            }
        }
    }

    SearchField {
        id: search
        readonly property bool floats: bar.rail && bar.searching
        z: 6
        floating: floats
        lensRight: floats && Theme.navRight
        fold: floats ? 0 : bar.fold
        x: floats ? (Theme.navRight ? top.x + searchSlot.x + searchSlot.width - width : top.x + searchSlot.x) : top.x + slot.x + ((Theme.sidebarRow - Theme.iconMd) / 2 - Theme.spaceSm) * bar.fold
        y: top.y + (floats ? searchSlot.y : head.y + (searchSlot.y - head.y) * bar.fold)
        width: floats ? Theme.fieldWidth : Math.max(Theme.sidebarRow, slot.width)
        dropWidth: floats ? 0 : top.width
        visible: floats || bar.fold < 1
        onBusyChanged: if (!busy) Qt.callLater(() => { if (!search.busy) bar.searching = false })
    }

    component Heading: Text {
        color: Theme.text
        font.pixelSize: Theme.fsHeadline
        font.weight: Font.DemiBold
    }

    component ListRow: SidebarRow {
        id: entry
        fold: bar.fold
        property var entity
        property string kind: "playlist"
        readonly property string page: kind === "playlist" || kind === "artist" ? kind : kind === "folder" ? "" : "album"
        icon: ({ playlist: "playlist", remote: "playlist", album: "disc", artist: "user", folder: "folder" })[kind]
        art: kind === "folder" ? "" : entity.thumb || ""
        text: entity.name || entity.title || ""
        active: Nav.view === page && Nav.params.id === entity.id && !!Nav.params.playlist === (kind === "remote")
        count: kind === "playlist" || kind === "folder" ? entity.count : 0
        mark: (kind === "playlist" || kind === "remote" || kind === "album") && Api.collectionDone(kind, entity.id)
        onClicked: {
            if (kind === "folder") Api.call("openFolder", { id: entity.id, open: !entity.open }, () => bar.refresh())
            else ({ playlist: Nav.playlist, remote: Nav.remote, album: Nav.album, artist: Nav.artist })[kind](entity.id)
        }
        menu: kind === "folder" ? folderMenu : spot

        MenuSpot { id: spot; kind: entry.kind; entity: entry.entity }

        DropArea {
            id: drop
            anchors.fill: parent
            enabled: entry.kind === "playlist"
            keys: ["tracks"]
            onDropped: Api.addToPlaylist(entry.entity, Nav.carried.tracks)
        }
        lit: drop.containsDrag

        RiffMenu {
            id: folderMenu
            MenuRow { text: qsTr("Rename"); glyph: "rename"; onTriggered: Nav.renamingFolder = entry.entity.id }
            MenuRow { text: qsTr("Delete folder"); glyph: "trash"; onTriggered: Api.call("deleteFolder", { id: entry.entity.id }, () => Api.libraryChanged()) }
        }

        TextField {
            id: rename
            anchors.fill: parent
            visible: entry.kind === "folder" && Nav.renamingFolder === entry.entity.id
            focus: visible
            leftPadding: Theme.spaceMd
            color: Theme.text
            font.pixelSize: Theme.fsBody
            background: Rectangle { radius: Theme.rMd; color: Theme.fillRaised }
            function begin() {
                if (!visible) return
                bar.renameField = rename
                text = bar.folderDraft || entry.entity.name
                if (!bar.folderDraft) selectAll()
                forceActiveFocus()
            }
            onVisibleChanged: begin()
            Component.onCompleted: begin()
            Keys.onReturnPressed: bar.finishRename()
            Keys.onEnterPressed: bar.finishRename()
            onActiveFocusChanged: if (!activeFocus) Qt.callLater(bar.renameBlurred)
            onTextEdited: bar.folderDraft = text
            Keys.onEscapePressed: { bar.folderDraft = ""; Nav.renamingFolder = null }
        }
    }

    SidebarRow {
        id: settings
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom; margins: Theme.spaceMd }
        fold: bar.fold
        icon: "settings"
        text: qsTr("Settings")
        active: Nav.view === "settings"
        quiet: true
        dot: Api.updateReady
        onClicked: Nav.go("settings")
    }
}
