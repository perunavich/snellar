pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})

    Fetch { id: res; method: "downloadedSongs"; args: ({ single: true }); live: true }
    Connections {
        target: Api
        function onDownloadedIdsChanged() { res.reload(true) }
        function onCollectionsChanged() { res.reload(true) }
    }

    Sorter {
        id: sorter
        name: "downloaded"
        items: res.data || []
        options: [{ key: "added", label: qsTr("Date downloaded") }].concat(sorter.trackKeys)
    }

    TrackPage {
        anchors.fill: parent
        dated: true
        tracks: sorter.shown
        from: "downloadedSongs"
        intro: Column {
            spacing: Theme.sectionGap

            Hero {
                width: parent.width
                glyph: "download"
                kind: qsTr("Downloaded")
                title: qsTr("Songs")
                subtitle: res.data ? Words.songs(res.data) : ""
                tools: ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

                PlayButtons { tracks: sorter.shown; from: "downloadedSongs" }
            }

            ListSkeleton { width: parent.width; visible: res.loading && !res.data }
            ErrorState { width: parent.width; fetch: res }

            EmptyState {
                visible: !!res.data && res.data.length === 0 && !res.code
                width: parent.width
                icon: "download"
                text: qsTr("No single songs downloaded")
                detail: qsTr("Choose Download in a song's menu.")
            }
            NoMatch { sorter: sorter; width: parent.width }
        }
    }
}
