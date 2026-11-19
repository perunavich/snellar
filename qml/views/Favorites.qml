pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    property var params: ({})

    Fetch { id: res; method: "favorites"; args: ({}); live: true }

    Sorter {
        id: sorter
        name: "favorites"
        items: res.data || []
        options: [{ key: "added", label: qsTr("Date added") }].concat(sorter.trackKeys)
    }

    TrackPage {
        anchors.fill: parent
        dated: true
        tracks: sorter.shown
        from: "favorites"
        intro: Column {
            spacing: Theme.sectionGap

            Hero {
                width: parent.width
                glyph: "heart"
                title: qsTr("Favorites")
                subtitle: res.data ? Words.songs(res.data) : ""
                tools: ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

                PlayButtons { tracks: sorter.shown; from: "favorites" }
            }

            ListSkeleton { width: parent.width; visible: res.loading && !res.data }
            ErrorState { width: parent.width; fetch: res }

            EmptyState { visible: !!res.data && res.data.length === 0 && !res.code; width: parent.width; icon: "heart"; text: qsTr("No favorites yet") }
            NoMatch { sorter: sorter; width: parent.width }
        }
    }
}
