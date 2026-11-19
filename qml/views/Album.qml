pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var album: res.data
    readonly property string from: album ? "album:" + album.id : ""
    readonly property var origin: Api.origin(remote ? "remote" : "album", album)
    property bool saved: false
    property Item barHead: null
    readonly property string barTitle: album ? album.title : ""

    readonly property bool remote: !!view.params.playlist
    Fetch { id: res; method: view.remote ? "remotePlaylist" : "album"; args: ({ id: view.params.id }); ready: !!view.params.id }

    function checkSaved() {
        if (!album) return
        const id = album.id
        Api.call(remote ? "savedPlaylists" : "savedAlbums", {}, v => { if (view) view.saved = (v || []).some(a => a.id === id) })
    }
    onAlbumChanged: checkSaved()
    Connections { target: Api; function onLibraryChanged() { view.checkSaved() } }

    TrackPage {
        anchors.fill: parent
        numbered: true
        showAlbum: false
        from: view.from
        origin: view.origin
        tracks: view.album ? view.album.tracks : []
        intro: Column {
            spacing: Theme.sectionGap

            Hero {
                id: hero
                width: parent.width
                visible: !!view.album
                url: view.album ? view.album.thumb : ""
                kind: view.album ? Words.kind(view.album.kind) : ""
                title: view.album ? view.album.title : ""
                artists: view.album ? view.album.artists : []
                subtitle: view.album ? [view.album.year, Words.songs(view.album.tracks)].filter(x => x).join(" · ") : ""
                Component.onCompleted: view.barHead = hero

                PlayButtons { tracks: view.album ? view.album.tracks : []; from: view.from; origin: view.origin }
                ActionButton {
                    text: view.saved ? qsTr("Saved") : qsTr("Save")
                    icon: view.saved ? "check" : "plus"
                    on: view.saved
                    onClicked: view.remote ? Api.toggleRemote(view.album, v => { if (view) view.saved = v })
                                           : Api.call("toggleAlbum", { album: view.album }, v => { if (view) view.saved = v; Api.libraryChanged() })
                }
                DownloadButton { kind: view.remote ? "remote" : "album"; entity: view.album }
                MoreButton { page: true; kind: view.remote ? "remote" : "album"; entity: view.album }
            }

            ListSkeleton { width: parent.width; hero: true; visible: res.loading && !view.album }

            ErrorState { width: parent.width; fetch: res }
        }
    }
}
