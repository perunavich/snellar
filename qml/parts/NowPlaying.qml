import QtQuick
import Snellar

Item {
    id: np

    property bool open: false
    readonly property real p: spring.value
    readonly property bool covering: p >= Motion.settledAt
    readonly property real hiding: Math.max(0, Math.min(1, (p - Motion.nowHandover) / (Motion.settledAt - Motion.nowHandover)))
    readonly property var track: Api.current
    readonly property Item controls: deck
    readonly property Item soundButton: sound

    visible: p > 0.002
    opacity: Math.min(1, p * 1.4)

    Component.onCompleted: Qt.callLater(() => open = Qt.binding(() => Nav.nowPlaying))

    SpringValue {
        id: spring
        zeta: np.open ? Motion.sheetOpenDamping : Motion.sheetCloseDamping
        response: np.open ? Motion.sheetOpenResponse : Motion.sheetCloseResponse
        target: np.open ? 1 : 0
    }

    Rectangle { anchors.fill: parent; color: Theme.window }

    Ambient {
        anchors.fill: parent
        source: np.track ? Theme.art(np.track.thumb, Theme.ambientPx) : ""
    }

    Rectangle { anchors.fill: parent; color: Theme.scrim }

    MouseArea { anchors.fill: parent }

    Item {
        id: stage
        anchors.fill: parent
        scale: 0.92 + 0.08 * np.p
        transform: Translate { y: (1 - np.p) * Theme.nowRise }

        IconButton {
            x: Theme.spaceXl
            y: Theme.spaceXl
            icon: "down"
            tip: qsTr("Close")
            size: Theme.controlLg
            iconSize: Theme.iconLg
            filledBg: true
            onClicked: Nav.nowPlaying = false
        }

        Column {
            id: deck
            readonly property real side: Theme.snap(Math.min(stage.width * 0.3, stage.height * 0.46, Theme.nowCover))
            width: side
            spacing: Theme.spaceLg
            x: Theme.snap(Math.max(Theme.controlXl, stage.width * 0.08))
            anchors.verticalCenter: parent.verticalCenter

            Cover {
                width: deck.side
                height: deck.side
                radius: Theme.rArtL
                px: Theme.coverNowPx
                standIn: true
                url: np.track ? np.track.thumb : ""
                scale: rest.value

                SpringValue {
                    id: rest
                    reducible: true
                    zeta: Motion.sheetOpenDamping
                    response: Motion.sheetOpenResponse
                    target: Api.playing ? 1 : Motion.pausedCover
                }
            }

            Column {
                width: parent.width
                spacing: Theme.spaceXs
                Text {
                    width: parent.width
                    text: np.track ? np.track.title : ""
                    color: Theme.text
                    font.family: Theme.font
                    font.pixelSize: Theme.fsLargeTitle
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight

                    AlbumLink { track: np.track }
                }
                ArtistLinks {
                    width: parent.width
                    artists: np.track ? np.track.artists : []
                    fontSize: Theme.fsBody
                }
                StreamBadge { width: parent.width }
            }

            PlayTrace { width: parent.width }

            SeekBar { width: parent.width }

            Item {
                width: parent.width
                height: Theme.rowHeight
                TransportControls { anchors.centerIn: parent }
            }

            Row {
                anchors.horizontalCenter: parent.horizontalCenter
                spacing: Theme.spaceSm
                LikeButton { }
                QuickAdd { }
                SoundButton { id: sound }
                MoreButton { size: Theme.controlMd; visible: !!np.track; entity: np.track }
                VolumeControl { anchors.verticalCenter: parent.verticalCenter; barWidth: Theme.volumeWide }
            }
        }

        NowTabs {
            anchors {
                left: deck.right
                leftMargin: Theme.snap(Math.max(Theme.controlXl, stage.width * 0.06))
                right: parent.right
                rightMargin: Theme.space2xl + Theme.spaceLg
                top: parent.top
                topMargin: Theme.spaceXl + (Theme.controlLg - Theme.segmentHeight) / 2
                bottom: parent.bottom
                bottomMargin: Theme.space2xl
            }
            lineSize: Theme.fsLyric
        }
    }
}
