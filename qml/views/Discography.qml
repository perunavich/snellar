pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: view
    property var params: ({})
    property string filter: params.kind || "albums"
    property string sort: "default"
    readonly property var disco: res.data
    readonly property var shown: {
        if (!disco) return []
        const list = filter === "albums" ? disco.albums : filter === "singles" ? disco.singles : disco.albums.concat(disco.singles)
        const year = a => parseInt(a.year) || 0
        const ordered = list.map((a, i) => ({ a: a, i: i }))
        const by = {
            newest: (x, y) => (year(y.a) - year(x.a)) || (x.i - y.i),
            oldest: (x, y) => ((year(x.a) || 9999) - (year(y.a) || 9999)) || (x.i - y.i),
            title: (x, y) => x.a.title.localeCompare(y.a.title) || (x.i - y.i)
        }[sort]
        if (by) ordered.sort(by)
        return ordered.map(x => x.a)
    }

    Fetch { id: res; method: "discography"; args: ({ id: view.params.id }); ready: !!view.params.id }

    PageScroll {
        anchors.fill: parent

        Column {
            width: parent.width
            spacing: Theme.spaceXs
            Text {
                width: parent.width
                text: qsTr("Discography")
                color: Theme.text
                font.family: Theme.font
                font.pixelSize: Theme.fsLargeTitle
                font.weight: Font.DemiBold
                elide: Text.ElideRight
            }
            Link {
                visible: !!view.disco
                text: view.disco ? view.disco.artist.name : ""
                onClicked: Nav.artist(view.disco.artist.id)
            }
        }

        Flow {
            width: parent.width
            spacing: Theme.spaceMd
            IosSegment {
                width: Theme.segmentWide
                current: view.filter
                options: [{ key: "all", label: qsTr("All") }, { key: "albums", label: qsTr("Albums") }, { key: "singles", label: qsTr("Singles & EPs") }]
                onPicked: key => view.filter = key
            }
            IosSegment {
                width: Theme.segmentWide
                current: view.sort
                options: [{ key: "default", label: qsTr("Default") }, { key: "newest", label: qsTr("Newest") }, { key: "oldest", label: qsTr("Oldest") }, { key: "title", label: qsTr("A–Z") }]
                onPicked: key => view.sort = key
            }
        }

        ListSkeleton { width: parent.width; visible: res.loading && !view.disco }
        ErrorState { width: parent.width; fetch: res }

        Text {
            visible: !!view.disco
            text: qsTr("%n release(s)", "", view.shown.length)
            color: Theme.text2
            font.pixelSize: Theme.fsCallout
        }

        Flow {
            width: parent.width
            spacing: Theme.spaceXl
            Repeater {
                model: view.shown
                delegate: AlbumCard { required property var modelData; album: modelData }
            }
        }
    }
}
