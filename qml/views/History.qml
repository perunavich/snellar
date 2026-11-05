pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    property var params: ({})

    Fetch { id: res; method: "history"; args: ({ limit: 100 }); live: true }

    Connections {
        target: Api
        function onHistoryChanged() { res.reload(true) }
    }

    Sorter {
        id: sorter
        name: "history"
        items: res.data || []
        options: [{ key: "recent", label: qsTr("Recently played") }].concat(sorter.trackKeys)
    }

    TrackPage {
        anchors.fill: parent
        dated: true
        radio: true
        tracks: sorter.shown
        intro: Column {
            spacing: Theme.sectionGap

            Row {
                width: parent.width
                spacing: Theme.spaceXl
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("History")
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: Theme.fsLargeTitle
                    font.weight: Font.DemiBold
                }
                PlayButtons { anchors.verticalCenter: parent.verticalCenter; visible: sorter.shown.length > 0; tracks: sorter.shown }
            }

            ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

            ListSkeleton { width: parent.width; visible: res.loading && !res.data }
            ErrorState { width: parent.width; fetch: res }

            EmptyState { visible: !!res.data && res.data.length === 0 && !res.code; width: parent.width; icon: "history"; text: qsTr("Nothing played yet") }
            NoMatch { sorter: sorter; width: parent.width }
        }
    }
}
