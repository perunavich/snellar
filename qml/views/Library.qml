pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    property string filter: params.filter || "all"
    readonly property var kinds: ({ all: ["playlist", "remote", "album", "artist"], playlists: ["playlist", "remote"], albums: ["album"], artists: ["artist"], downloads: [] })

    Fetch { id: res; method: "library"; live: true }
    Fetch { id: files; method: "downloads"; live: true }

    Sorter {
        id: sorter
        name: "library"
        items: (res.data || []).filter(e => view.kinds[view.filter].indexOf(e.kind) >= 0).map(e => Object.assign({}, e.item, { type: e.kind, at: e.at }))
        options: [{ key: "added", label: qsTr("Recently added") }, { key: "title", label: qsTr("Title") }, { key: "artist", label: qsTr("Creator") }]
    }

    PageScroll {
        anchors.fill: parent

        Text {
            text: qsTr("Library")
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsLargeTitle
            font.weight: Font.DemiBold
        }

        Flow {
            width: parent.width
            spacing: Theme.spaceMd
            IosSegment {
                width: Math.max(Theme.filterWidth, fit)
                current: view.filter
                options: [{ key: "all", label: qsTr("All") }, { key: "playlists", label: qsTr("Playlists") }, { key: "albums", label: qsTr("Albums") }, { key: "artists", label: qsTr("Artists") }, { key: "downloads", label: qsTr("Downloads") }]
                onPicked: key => view.filter = key
            }
            Row {
                spacing: Theme.spaceMd
                ListTools { anchors.verticalCenter: parent.verticalCenter; sorter: sorter; visible: view.filter !== "downloads" }
                PillButton {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("Import playlist")
                    icon: "download"
                    onClicked: Nav.go("import")
                }
            }
        }

        ErrorState { width: parent.width; fetch: res }
        EmptyState { visible: !!res.data && !res.code && sorter.items.length === 0 && view.filter !== "downloads"; width: parent.width; icon: "library"; text: qsTr("Nothing here yet") }
        EmptyState { visible: view.filter === "downloads" && !!files.data && files.data.length === 0; width: parent.width; icon: "download"; text: qsTr("Nothing downloaded yet") }
        NoMatch { sorter: sorter; width: parent.width }

        Column {
            width: parent.width
            visible: view.filter === "downloads"

            Repeater {
                model: files.data || []
                delegate: Item {
                    id: download
                    required property var modelData
                    width: parent.width
                    height: Theme.controlXl

                    Text {
                        anchors { left: parent.left; right: size.left; rightMargin: Theme.spaceLg; verticalCenter: parent.verticalCenter }
                        text: download.modelData.name
                        color: Theme.text
                        font.pixelSize: Theme.fsBody
                        elide: Text.ElideRight
                    }
                    Text {
                        id: size
                        anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                        text: qsTr("%1 MB").arg((download.modelData.size / 1048576).toFixed(1))
                        color: Theme.text2
                        font.pixelSize: Theme.fsCallout
                    }
                }
            }
        }

        Flow {
            width: parent.width
            spacing: Theme.spaceXl
            Repeater {
                model: sorter.shown
                delegate: Loader {
                    id: slot
                    required property var modelData
                    sourceComponent: modelData.type === "artist" ? artist : modelData.type === "playlist" ? playlist : album
                    Component { id: artist; ArtistCard { artist: slot.modelData } }
                    Component { id: playlist; PlaylistCard { playlist: slot.modelData } }
                    Component { id: album; AlbumCard { album: slot.modelData } }
                }
            }
        }
    }
}
