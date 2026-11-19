pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    readonly property var songs: res.data
    readonly property string from: "songs:" + (params.id || "")

    Fetch { id: res; method: "artistSongs"; args: ({ id: view.params.id }); ready: !!view.params.id }
    Fetch { id: who; method: "artist"; args: ({ id: view.params.id }); ready: !!view.params.id }

    TrackPage {
        anchors.fill: parent
        numbered: true
        from: view.from
        tracks: view.songs || []
        intro: Column {
            spacing: Theme.sectionGap

            Hero {
                width: parent.width
                url: who.data ? who.data.thumb : ""
                title: qsTr("Top songs")
                artists: who.data ? [{ id: who.data.id, name: who.data.name }] : []
                subtitle: view.songs ? Words.songs(view.songs) : ""

                PlayButtons { visible: res.loading || (!!view.songs && view.songs.length > 0); tracks: view.songs || []; from: view.from }
            }

            ListSkeleton { width: parent.width; visible: res.loading && !view.songs }
            ErrorState { width: parent.width; fetch: res }
        }
    }
}
