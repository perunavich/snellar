import QtQuick
import Snellar

Item {
    id: column
    readonly property var track: Api.current
    readonly property Item soundButton: sound
    readonly property real glyphInset: (Theme.controlSm - Theme.iconMd) / 2

    SidePanel {
        anchors.fill: parent
        lineRight: Theme.navRight
    }

    Column {
        id: deck
        x: Theme.spaceXl
        y: Theme.spaceXl
        width: parent.width - 2 * Theme.spaceXl
        spacing: Theme.spaceMd

        Cover {
            width: parent.width
            height: parent.width
            radius: Theme.rArtL
            px: Theme.coverColumnPx
            standIn: true
            url: column.track ? column.track.thumb : ""

            Tap { enabled: !!column.track; onTapped: Nav.nowPlaying = true }
            MenuTap { menu: spot }
        }

        Item {
            width: parent.width
            height: info.height

            MenuSpot { id: spot; entity: column.track }

            MoreButton {
                id: more
                anchors { right: parent.right; rightMargin: -column.glyphInset; verticalCenter: parent.verticalCenter }
                visible: !!column.track
                menu: spot
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

                    AlbumLink { track: column.track }
                }
                ArtistLinks {
                    width: parent.width
                    artists: column.track ? column.track.artists : []
                }
                Text {
                    objectName: "why"
                    width: parent.width
                    visible: text.length > 0
                    text: column.track && column.track.pick ? Words.why(column.track.pick.why) : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsCaption
                    elide: Text.ElideRight
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
                x: -column.glyphInset
                anchors.verticalCenter: parent.verticalCenter
                spacing: Theme.spaceXs

                LikeButton { size: Theme.controlSm }
                QuickAdd { size: Theme.controlSm }
                SoundButton { id: sound; size: Theme.controlSm }
                IconButton { visible: !!column.track; size: Theme.controlSm; icon: "expand"; tip: qsTr("Now Playing"); onClicked: Nav.nowPlaying = true }
            }

            VolumeControl {
                id: volume
                anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                buttonSize: Theme.controlSm
                barWidth: parent.width + column.glyphInset - actions.implicitWidth - Theme.spaceMd - volume.buttonSize - volume.spacing
            }
        }
    }

    NowTabs {
        anchors { left: deck.left; right: deck.right; top: deck.bottom; bottom: parent.bottom; topMargin: Theme.spaceLg; bottomMargin: Theme.spaceMd }
    }
}
