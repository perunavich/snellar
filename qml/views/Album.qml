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

    readonly property bool remote: !!view.params.playlist
    Fetch { id: res; method: view.remote ? "remotePlaylist" : "album"; args: ({ id: view.params.id }); ready: !!view.params.id }

    function checkSaved() { if (album) Api.call(remote ? "savedPlaylists" : "savedAlbums", {}, v => view.saved = (v || []).some(a => a.id === view.album.id)) }
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
                width: parent.width
                visible: !!view.album
                url: view.album ? view.album.thumb : ""
                kind: view.album ? Words.kind(view.album.kind) : ""
                title: view.album ? view.album.title : ""
                artists: view.album ? view.album.artists : []
                subtitle: view.album ? [view.album.year, qsTr("%n song(s)", "", view.album.tracks.length)].filter(x => x).join(" · ") : ""

                PlayButtons { tracks: view.album ? view.album.tracks : []; from: view.from; origin: view.origin }
                PillButton {
                    text: view.saved ? qsTr("Saved") : qsTr("Save")
                    icon: view.saved ? "check" : "plus"
                    onClicked: view.remote ? Api.toggleRemote(view.album, v => view.saved = v)
                                           : Api.call("toggleAlbum", { album: view.album }, v => { view.saved = v; Api.libraryChanged() })
                }
                IconButton {
                    id: more
                    icon: "more"
                    tip: qsTr("More")
                    size: Theme.controlLg
                    filledBg: true
                    onClicked: menu.open(more, 0, more.height)
                    MenuSpot { id: menu; kind: view.remote ? "remote" : "album"; entity: view.album }
                }
            }

            ListSkeleton { width: parent.width; visible: res.loading && !view.album }

            ErrorState { width: parent.width; fetch: res }
        }
    }
}
