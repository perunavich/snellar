pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var list: res.data
    readonly property string from: list ? "playlist:" + list.id : ""
    readonly property var origin: Api.origin("playlist", list)
    property bool renaming: false
    property Item barHead: null
    readonly property string barTitle: list ? list.name : ""

    function asked() {
        if (!list) return
        if (Nav.renaming === list.id) {
            Nav.renaming = null
            renaming = true
        }
        if (Nav.editing === list.id) {
            Nav.editing = null
            editor.edit(list)
        }
    }
    onListChanged: Qt.callLater(asked)
    onRenamingChanged: if (renaming) page.contentY = page.originY
    Connections {
        target: Nav
        function onRenamingChanged() { view.asked() }
        function onEditingChanged() { view.asked() }
    }

    function keep(tracks) {
        res.data = Object.assign({}, list, { tracks: tracks, count: tracks.length, thumb: list.cover || (tracks.length ? tracks[0].thumb : "") })
    }

    Connections {
        target: Api
        function onPlaylistTracksRemoved(id, trackIds) {
            if (view.list && view.list.id === id) view.keep(view.list.tracks.filter(t => trackIds.indexOf(t.id) < 0))
        }
    }

    Fetch { id: res; method: "playlist"; args: ({ id: view.params.id }); ready: view.params.id !== undefined; live: true }

    Sorter {
        id: sorter
        name: "playlist"
        items: view.list ? view.list.tracks : []
        options: [{ key: "custom", label: qsTr("Custom order") }, { key: "added", label: qsTr("Date added") }].concat(sorter.trackKeys)
    }

    function move(track, to) {
        const tracks = list.tracks.filter(t => t.id !== track.id)
        tracks.splice(to, 0, list.tracks.find(t => t.id === track.id))
        keep(tracks)
        Api.call("movePlaylistItem", { id: list.id, trackId: track.id, to: to }, () => Api.libraryChanged())
    }

    function commit(text) {
        const name = text.trim()
        renaming = false
        if (name && name !== list.name) Api.call("renamePlaylist", { id: list.id, name: name }, () => { res.reload(); Api.libraryChanged() })
    }

    PlaylistEditor {
        id: editor
        onSaved: Api.libraryChanged()
    }

    TrackPage {
        id: page
        anchors.fill: parent
        dated: true
        playlistId: view.list ? view.list.id : null
        from: view.from
        origin: view.origin
        tracks: sorter.shown
        reorderable: sorter.custom
        onMoveRequested: (t, to) => view.move(t, to)
        intro: Column {
            spacing: Theme.sectionGap

            Hero {
                id: hero
                width: parent.width
                visible: !!view.list
                url: view.list ? view.list.thumb : ""
                kind: qsTr("Playlist")
                title: view.list ? view.list.name : ""
                subtitle: view.list ? Words.songs(view.list.tracks) : ""
                description: view.list ? view.list.description : ""
                editingTitle: view.renaming
                onTitleDoubleTapped: view.renaming = true
                Component.onCompleted: view.barHead = hero
                titleEditor: TextField {
                    anchors.fill: parent
                    padding: 0
                    verticalAlignment: TextInput.AlignVCenter
                    color: Theme.text
                    placeholderText: qsTr("Playlist name")
                    placeholderTextColor: Theme.text3
                    selectByMouse: true
                    font.family: Theme.font
                    font.pixelSize: Theme.fsHero
                    font.weight: Font.Bold
                    background: Rectangle {
                        anchors { fill: parent; leftMargin: -Theme.spaceSm; rightMargin: -Theme.spaceSm }
                        radius: Theme.rMd
                        color: Theme.fillRaised
                    }
                    onVisibleChanged: if (visible) { text = view.list.name; selectAll(); forceActiveFocus() }
                    onAccepted: view.commit(text)
                    onActiveFocusChanged: if (!activeFocus && view.renaming) view.commit(text)
                    Keys.onEscapePressed: view.renaming = false
                }

                tools: ListTools { sorter: sorter; visible: !!view.list && view.list.count > 0 }

                PlayButtons { tracks: sorter.shown; from: view.from; origin: view.origin }
                ActionButton { id: editButton; text: qsTr("Edit details"); icon: "edit"; onClicked: editor.edit(view.list, editButton) }
                DownloadButton { kind: "playlist"; entity: view.list }
                MoreButton { page: true; kind: "playlist"; entity: view.list }
            }

            ListSkeleton { width: parent.width; hero: true; visible: res.loading && !view.list }
            ErrorState { width: parent.width; fetch: res }
            EmptyState {
                visible: !!view.list && view.list.count === 0 && !res.code
                width: parent.width
                icon: "playlist"
                text: qsTr("No songs yet")
                detail: qsTr("Choose “Add to playlist” in a song's menu.")
            }
            NoMatch { sorter: sorter; width: parent.width }
        }
    }
}
