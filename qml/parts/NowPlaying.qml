import QtQuick
import Snellar

Item {
    id: np

    property bool open: false
    readonly property real p: spring.value
    readonly property bool covering: p > 0.98
    readonly property var track: Api.current

    visible: p > 0.002
    opacity: Math.min(1, p * 1.4)

    Component.onCompleted: Qt.callLater(() => open = Qt.binding(() => Nav.nowPlaying))

    SpringValue {
        id: spring
        zeta: np.open ? Theme.springBase[0] : Theme.springCritical[0]
        response: np.open ? Theme.springBase[1] : Theme.springCritical[1]
        target: np.open ? 1 : 0
    }

    Rectangle { anchors.fill: parent; color: Theme.window }

    Ambient {
        anchors.fill: parent
        source: np.track ? Theme.art(np.track.thumb, 160) : ""
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
                px: 800
                url: np.track ? np.track.thumb : ""
                scale: Api.playing ? 1 : 0.9
                Behavior on scale { NumberAnimation { duration: Theme.emphasized; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }
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

                    TapHandler {
                        enabled: Api.albumOf(np.track).length > 0
                        onTapped: {
                            Nav.nowPlaying = false
                            Nav.album(Api.albumOf(np.track))
                        }
                    }
                    HoverHandler { enabled: Api.albumOf(np.track).length > 0; cursorShape: Qt.PointingHandCursor }
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
                IconButton {
                    icon: Api.liked ? "heartFill" : "heart"
                    tip: Api.liked ? qsTr("Unlike") : qsTr("Like")
                    active: Api.liked
                    onClicked: Api.like(np.track)
                }
                IconButton { icon: "sliders"; tip: qsTr("Sound"); active: Nav.audio; onClicked: Nav.audio = !Nav.audio }
                IconButton {
                    id: more
                    icon: "more"
                    tip: qsTr("More")
                    visible: !!np.track
                    onClicked: menu.open(more, 0, more.height)
                    MenuSpot { id: menu; entity: np.track }
                }
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
