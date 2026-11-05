import QtQuick
import Snellar

Item {
    property var params: ({})

    Fetch { id: res; method: "followed"; live: true }

    Sorter {
        id: sorter
        name: "artists"
        items: res.data || []
        options: [{ key: "added", label: qsTr("Recently added") }, { key: "title", label: qsTr("Name") }]
    }

    PageScroll {
        anchors.fill: parent

        Text {
            text: qsTr("Artists")
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsLargeTitle
            font.weight: Font.DemiBold
        }

        ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

        ErrorState { width: parent.width; fetch: res }
        EmptyState { visible: !!res.data && res.data.length === 0 && !res.code; width: parent.width; icon: "user"; text: qsTr("Follow artists to see them here") }
        NoMatch { sorter: sorter; width: parent.width }

        Flow {
            width: parent.width
            spacing: Theme.spaceXl
            Repeater {
                model: sorter.shown
                delegate: ArtistCard { required property var modelData; artist: modelData }
            }
        }
    }
}
