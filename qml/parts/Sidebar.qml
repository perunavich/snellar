pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Rectangle {
    id: bar
    color: Theme.sidebar

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

    function focusSearch() { search.focusField() }

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

    Component.onCompleted: refresh()
    Connections { target: Api; function onLibraryChanged() { bar.refresh() } }

    Rectangle {
        anchors { top: parent.top; bottom: parent.bottom }
        x: Theme.navRight ? 0 : parent.width - width
        width: 1
        color: Theme.separator
    }

    Column {
        id: top
        z: 5
        anchors { left: parent.left; right: parent.right; top: parent.top; margins: Theme.spaceMd }
        spacing: Theme.spaceXs

        SearchField { id: search; width: parent.width; z: 5 }

        Item { width: 1; height: Theme.spaceSm }

        SidebarRow { width: parent.width; icon: "home"; text: qsTr("Home"); active: Nav.view === "discover"; onClicked: Nav.go("discover") }

        Item {
            width: parent.width
            height: Theme.controlMd

            Text {
                anchors { left: parent.left; leftMargin: Theme.spaceSm; verticalCenter: library.verticalCenter }
                text: qsTr("Library")
                color: Nav.view === "library" ? Theme.accent : Theme.text3
                font.pixelSize: Theme.fsCaption
                TapHandler { onTapped: Nav.go("library") }
                HoverHandler { cursorShape: Qt.PointingHandCursor }
            }
            IconButton {
                id: library
                anchors { right: parent.right; bottom: parent.bottom }
                icon: "library"
                tip: qsTr("Whole library")
                size: Theme.sidebarRow
                iconSize: Theme.iconSm
                active: Nav.view === "library"
                onClicked: Nav.go("library")
            }
        }
        SidebarRow { width: parent.width; icon: "heart"; text: qsTr("Favorites"); active: Nav.view === "favorites"; count: bar.counts.favorites; onClicked: Nav.go("favorites") }
        SidebarRow { width: parent.width; icon: "disc"; text: qsTr("Albums"); active: Nav.view === "albums"; count: bar.counts.albums; onClicked: Nav.go("albums") }
        SidebarRow { width: parent.width; icon: "user"; text: qsTr("Artists"); active: Nav.view === "artists"; count: bar.counts.artists; onClicked: Nav.go("artists") }
        SidebarRow { width: parent.width; icon: "history"; text: qsTr("History"); active: Nav.view === "history"; onClicked: Nav.go("history") }

        Text { visible: bar.pinned.length > 0; text: qsTr("Pinned"); color: Theme.text3; font.pixelSize: Theme.fsCaption; topPadding: Theme.spaceMd; leftPadding: Theme.spaceSm }
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
            height: Theme.controlMd

            Text {
                anchors { left: parent.left; leftMargin: Theme.spaceSm; verticalCenter: addPlaylist.verticalCenter }
                text: qsTr("Playlists")
                color: Theme.text3
                font.pixelSize: Theme.fsCaption
            }
            IconButton {
                id: addPlaylist
                anchors { right: parent.right; bottom: parent.bottom }
                icon: bar.creating ? "close" : "plus"
                tip: bar.creating ? qsTr("Cancel") : qsTr("New playlist")
                size: Theme.sidebarRow
                iconSize: Theme.iconSm
                onClicked: {
                    bar.creating = !bar.creating
                    if (bar.creating) field.forceActiveFocus()
                }
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

        Link {
            width: parent.width
            visible: bar.creating
            leftPadding: Theme.spaceMd
            text: qsTr("Import from a link or CSV")
            onClicked: {
                bar.creating = false
                Nav.go("import")
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

    component ListRow: SidebarRow {
        id: entry
        property var entity
        property string kind: "playlist"
        readonly property string page: kind === "playlist" || kind === "artist" ? kind : kind === "folder" ? "" : "album"
        icon: ({ playlist: "playlist", remote: "playlist", album: "disc", artist: "user", folder: "folder" })[kind]
        art: kind === "folder" ? "" : entity.thumb || ""
        text: entity.name || entity.title || ""
        active: Nav.view === page && Nav.params.id === entity.id && !!Nav.params.playlist === (kind === "remote")
        count: kind === "playlist" || kind === "folder" ? entity.count : 0
        onClicked: {
            if (kind === "folder") Api.call("openFolder", { id: entity.id, open: !entity.open }, () => bar.refresh())
            else ({ playlist: Nav.playlist, remote: Nav.remote, album: Nav.album, artist: Nav.artist })[kind](entity.id)
        }
        onMenuRequested: (x, y) => kind === "folder" ? folderMenu.popup(entry, x, y) : menu.open(entry, x, y)

        MenuSpot { id: menu; kind: entry.kind; entity: entry.entity }

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
            MenuRow { text: qsTr("Rename"); onTriggered: Nav.renamingFolder = entry.entity.id }
            MenuRow { text: qsTr("Delete folder"); onTriggered: Api.call("deleteFolder", { id: entry.entity.id }, () => Api.libraryChanged()) }
        }

        TextField {
            id: rename
            anchors.fill: parent
            visible: entry.kind === "folder" && Nav.renamingFolder === entry.entity.id
            focus: visible
            leftPadding: Theme.spaceMd
            color: Theme.text
            font.pixelSize: Theme.fsCallout
            background: Rectangle { radius: Theme.rRow; color: Theme.fillRaised }
            function begin() {
                if (!visible) return
                bar.renameField = rename
                text = bar.folderDraft || entry.entity.name
                if (!bar.folderDraft) selectAll()
                forceActiveFocus()
            }
            onVisibleChanged: begin()
            Component.onCompleted: begin()
            onAccepted: bar.finishRename()
            onActiveFocusChanged: if (!activeFocus) Qt.callLater(bar.renameBlurred)
            onTextEdited: bar.folderDraft = text
            Keys.onEscapePressed: { bar.folderDraft = ""; Nav.renamingFolder = null }
        }
    }

    SidebarRow {
        id: settings
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom; margins: Theme.spaceMd }
        icon: "settings"
        text: qsTr("Settings")
        active: Nav.view === "settings"
        onClicked: Nav.go("settings")
    }
}
