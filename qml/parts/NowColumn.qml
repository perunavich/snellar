import QtQuick
import Snellar

Item {
    id: column
    readonly property var track: Api.current

    Glass { anchors.fill: parent }

    Column {
        id: deck
        x: Theme.spaceXl
        y: Theme.spaceXl
        width: parent.width - 2 * Theme.spaceXl
        spacing: Theme.spaceMd

        Cover {
            id: art
            width: parent.width
            height: parent.width
            radius: Theme.rArtL
            px: 640
            url: column.track ? column.track.thumb : ""

            TapHandler { enabled: !!column.track; onTapped: Nav.nowPlaying = true }
            TapHandler {
                acceptedButtons: Qt.RightButton
                onTapped: p => menu.open(art, p.position.x, p.position.y)
            }
            HoverHandler { cursorShape: Qt.PointingHandCursor }
        }

        Item {
            width: parent.width
            height: info.height

            MenuSpot { id: menu; entity: column.track }

            IconButton {
                id: more
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                visible: !!column.track
                size: Theme.controlSm
                icon: "more"
                tip: qsTr("More")
                onClicked: menu.open(more, 0, more.height)
            }

            Column {
                id: info
                anchors { left: parent.left; right: more.left; rightMargin: Theme.spaceXs }
                spacing: Theme.spaceXs

                Marquee {
                    width: parent.width
                    text: column.track ? column.track.title : qsTr("Nothing playing")
                    color: Theme.text
                    font.pixelSize: Theme.fsTitle2
                    font.weight: Font.DemiBold
                    font.family: Theme.font

                    TapHandler { enabled: Api.albumOf(column.track).length > 0; onTapped: Nav.album(Api.albumOf(column.track)) }
                    HoverHandler { enabled: Api.albumOf(column.track).length > 0; cursorShape: Qt.PointingHandCursor }
                }
                ArtistLinks {
                    width: parent.width
                    artists: column.track ? column.track.artists : []
                }
                StreamBadge { width: parent.width }
            }
        }

        PlayTrace { width: parent.width }

        SeekBar { width: parent.width }

        Item {
            width: parent.width
            height: Theme.playPill
            TransportControls { anchors.centerIn: parent }
        }

        Item {
            width: parent.width
            height: Theme.controlSm

            Row {
                id: actions
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.spaceXs
                visible: !!column.track

                IconButton { size: Theme.controlSm; icon: Api.liked ? "heartFill" : "heart"; tip: Api.liked ? qsTr("Unlike") : qsTr("Like"); active: Api.liked; onClicked: Api.like(column.track) }
                IconButton { size: Theme.controlSm; icon: "download"; tip: qsTr("Download"); onClicked: Api.call("download", { track: column.track }) }
                IconButton { size: Theme.controlSm; icon: "sliders"; tip: qsTr("Sound"); active: Nav.audio; onClicked: Nav.audio = !Nav.audio }
                IconButton { size: Theme.controlSm; icon: "expand"; tip: qsTr("Now Playing"); onClicked: Nav.nowPlaying = true }
            }

            VolumeControl {
                id: volume
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                buttonSize: Theme.controlSm
                barWidth: parent.width - actions.implicitWidth - Theme.spaceMd - volume.buttonSize - volume.spacing
            }
        }
    }

    NowTabs {
        anchors { left: deck.left; right: deck.right; top: deck.bottom; bottom: parent.bottom; topMargin: Theme.spaceLg; bottomMargin: Theme.spaceMd }
    }
}
