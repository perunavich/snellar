import QtQuick
import Snellar

Item {
    property var params: ({})

    Fetch { id: res; method: "savedAlbums"; live: true }

    Sorter {
        id: sorter
        name: "albums"
        items: res.data || []
        options: [{ key: "added", label: qsTr("Recently added") }, { key: "title", label: qsTr("Title") }, { key: "artist", label: qsTr("Artist") }, { key: "year", label: qsTr("Year") }]
    }

    PageScroll {
        anchors.fill: parent

        Text {
            text: qsTr("Albums")
            color: Theme.text
            font.family: Theme.font
            font.pixelSize: Theme.fsLargeTitle
            font.weight: Font.DemiBold
        }

        ListTools { sorter: sorter; visible: !!res.data && res.data.length > 0 }

        ErrorState { width: parent.width; fetch: res }
        EmptyState { visible: !!res.data && res.data.length === 0 && !res.code; width: parent.width; icon: "disc"; text: qsTr("Save albums to see them here") }
        NoMatch { sorter: sorter; width: parent.width }

        Flow {
            width: parent.width
            spacing: Theme.spaceXl
            Repeater {
                model: sorter.shown
                delegate: AlbumCard { required property var modelData; album: modelData }
            }
        }
    }
}
