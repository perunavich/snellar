pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var turnedOn: Object.keys(Api.collections)
    readonly property var kept: Api.cards((saved.data || []).filter(e => Api.isKept(e)))
        .sort((a, b) => view.group(a) - view.group(b) || view.turnedOn.indexOf(b.type + ":" + b.id) - view.turnedOn.indexOf(a.type + ":" + a.id))
    readonly property var songs: every.data || []
    readonly property var singles: alone.data || []
    readonly property bool loaded: !!every.data && !!saved.data && !!alone.data

    function group(card) {
        if (card.type === "playlist") return 0
        if (card.type === "remote") return 1
        return ["single", "ep"].indexOf(String(card.kind).toLowerCase()) >= 0 ? 3 : 2
    }
    function about(card) {
        if (card.type === "playlist") return qsTr("Playlist · %n song(s)", "", card.count)
        const by = (card.artists || []).map(a => a.name).filter(n => n).join(", ")
        return [Words.kind(card.kind || "playlist"), by].filter(x => x).join(" · ")
    }

    Fetch { id: every; method: "downloadedSongs"; live: true }
    Fetch { id: alone; method: "downloadedSongs"; args: ({ single: true }); live: true }
    Fetch { id: saved; method: "library"; live: true }
    Connections {
        target: Api
        function onDownloadedIdsChanged() {
            every.reload(true)
            alone.reload(true)
        }
        function onCollectionsChanged() { alone.reload(true) }
    }

    PageScroll {
        anchors.fill: parent

        Hero {
            width: parent.width
            glyph: "download"
            title: qsTr("Downloaded")
            subtitle: every.data ? Words.songs(view.songs) : ""

            PlayButtons { tracks: view.songs; from: "downloaded" }
        }

        ListSkeleton { width: parent.width; visible: !view.loaded && (every.loading || saved.loading) }
        ErrorState { width: parent.width; fetch: every }

        EmptyState {
            visible: view.loaded && view.kept.length === 0 && view.singles.length === 0 && !every.code
            width: parent.width
            icon: "download"
            text: qsTr("Nothing downloaded yet")
            detail: qsTr("Choose Download in the menu of a song, playlist or album.")
        }

        Column {
            width: parent.width
            Repeater {
                model: view.kept
                delegate: CollectionRow {
                    required property var modelData
                    width: parent.width
                    kind: modelData.type
                    entity: modelData
                    url: modelData.thumb || ""
                    title: modelData.name || modelData.title || ""
                    subtitle: view.about(modelData)
                    onOpened: ({ playlist: Nav.playlist, remote: Nav.remote, album: Nav.album })[kind](modelData.id)
                }
            }
            CollectionRow {
                width: parent.width
                visible: view.singles.length > 0
                kind: "songs"
                glyph: "music"
                title: qsTr("Songs")
                subtitle: qsTr("%n song(s)", "", view.singles.length)
                onOpened: Nav.go("downloadedSongs")
            }
        }
    }
}
