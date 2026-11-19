import QtQuick
import Snellar

Item {
    id: capsule
    readonly property var track: Api.current
    readonly property Item soundButton: sound
    readonly property bool failed: !!Api.failure && !!track && Api.failure.id === track.id && !Api.loading

    readonly property real radius: shell.radius
    implicitHeight: Theme.capsuleHeight

    Glass { id: shell; anchors.fill: parent }

    Item {
        id: left
        anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        width: Math.max(Theme.capsuleInfo, capsule.width * 0.28)
        height: Theme.capsuleArt

        MenuTap { menu: spot }
        MenuSpot { id: spot; entity: capsule.track }

        Cover {
            id: art
            width: Theme.capsuleArt
            height: Theme.capsuleArt
            radius: Theme.rArtM
            px: Theme.coverCapsulePx
            standIn: true
            url: capsule.track ? capsule.track.thumb : ""
            Tap { enabled: !!capsule.track; gesturePolicy: TapHandler.DragThreshold; onTapped: Nav.nowPlaying = true }
        }

        Column {
            anchors { left: art.right; leftMargin: Theme.spaceMd; right: like.left; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }

            Marquee {
                width: parent.width
                text: capsule.track ? capsule.track.title : qsTr("Nothing playing")
                color: Theme.text
                font.pixelSize: Theme.fsHeadline
                font.weight: Font.DemiBold

                AlbumLink { track: capsule.track; gesturePolicy: TapHandler.DragThreshold }
            }
            ArtistLinks { width: parent.width; visible: !capsule.failed; artists: capsule.track ? capsule.track.artists : [] }
            Row {
                width: parent.width
                spacing: Theme.spaceXs
                visible: capsule.failed
                IconButton {
                    id: retry
                    anchors.verticalCenter: parent.verticalCenter
                    icon: "refresh"
                    tip: qsTr("Try again")
                    size: Theme.controlSm
                    iconSize: Theme.iconSm
                    onClicked: Api.call("jump", { index: Api.cursor })
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    width: parent.width - retry.width - parent.spacing
                    text: capsule.failed ? Errors.text(Api.failure.code) : ""
                    color: Theme.text2
                    font.pixelSize: Theme.fsCaption
                    maximumLineCount: 2
                    wrapMode: Text.WordWrap
                    elide: Text.ElideRight
                }
            }
        }

        LikeButton {
            id: like
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            size: Theme.controlSm
            iconSize: Theme.iconSm
        }
    }

    TransportControls {
        compact: true
        x: Math.round((parent.width - width) / 2)
        y: seek.y - height
    }

    SeekBar {
        id: seek
        x: Math.round((parent.width - width) / 2)
        anchors { bottom: parent.bottom; bottomMargin: Theme.spaceXs }
        width: Math.floor(Math.max(0, Math.min(Theme.capsuleCenter, 2 * (Math.min(capsule.width / 2 - left.x - left.width, right.x - capsule.width / 2) - Theme.spaceMd))))
    }

    Row {
        id: right
        anchors { right: parent.right; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        spacing: Theme.spaceXs

        QuickAdd { }
        IconButton { icon: "queue"; tip: qsTr("Queue"); onClicked: { Nav.tab = "next"; Nav.nowPlaying = true } }
        IconButton { icon: "lyrics"; tip: qsTr("Lyrics"); onClicked: { Nav.tab = "lyrics"; Nav.nowPlaying = true } }
        SoundButton { id: sound }
        VolumeControl {
            anchors.verticalCenter: parent.verticalCenter
            showBar: capsule.width >= Theme.capsuleWide
        }
    }
}
