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

            Column {
                width: parent.width
                spacing: Theme.spaceXs
                Text {
                    width: parent.width
                    text: qsTr("Popular songs")
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: Theme.fsLargeTitle
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                Link {
                    visible: !!who.data
                    text: who.data ? who.data.name : ""
                    onClicked: Nav.artist(view.params.id)
                }
            }

            Row {
                spacing: Theme.spaceLg
                PlayButtons { anchors.verticalCenter: parent.verticalCenter; tracks: view.songs || []; from: view.from }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !!view.songs
                    text: qsTr("%n song(s)", "", view.songs ? view.songs.length : 0)
                    color: Theme.text2
                    font.pixelSize: Theme.fsCallout
                }
            }

            ListSkeleton { width: parent.width; visible: res.loading && !view.songs }
            ErrorState { width: parent.width; fetch: res }
        }
    }
}
