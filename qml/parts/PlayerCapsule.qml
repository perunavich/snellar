import QtQuick
import Snellar

Item {
    id: capsule
    readonly property var track: Api.current
    readonly property bool failed: !!Api.failure && !!track && Api.failure.id === track.id && !Api.loading

    implicitHeight: Theme.capsuleHeight

    Glass { anchors.fill: parent; radius: height / 2 }

    Item {
        id: left
        anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        width: Math.max(Theme.capsuleInfo, capsule.width * 0.28)
        height: Theme.capsuleArt

        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: p => menu.open(left, p.position.x, p.position.y)
        }
        MenuSpot { id: menu; entity: capsule.track }

        Cover {
            id: art
            width: Theme.capsuleArt
            height: Theme.capsuleArt
            radius: Theme.rArtM
            px: 112
            url: capsule.track ? capsule.track.thumb : ""
            TapHandler { enabled: !!capsule.track; onTapped: Nav.nowPlaying = true }
            HoverHandler { cursorShape: Qt.PointingHandCursor }
        }

        Column {
            anchors { left: art.right; leftMargin: Theme.spaceMd; right: like.left; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }

            Marquee {
                width: parent.width
                text: capsule.track ? capsule.track.title : qsTr("Nothing playing")
                color: Theme.text
                font.pixelSize: Theme.fsHeadline
                font.weight: Font.DemiBold

                TapHandler { enabled: Api.albumOf(capsule.track).length > 0; onTapped: Nav.album(Api.albumOf(capsule.track)) }
                HoverHandler { enabled: Api.albumOf(capsule.track).length > 0; cursorShape: Qt.PointingHandCursor }
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

        IconButton {
            id: like
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            visible: !!capsule.track
            icon: Api.liked ? "heartFill" : "heart"
            tip: Api.liked ? qsTr("Unlike") : qsTr("Like")
            size: Theme.controlSm
            iconSize: Theme.iconSm
            active: Api.liked
            onClicked: Api.like(capsule.track)
        }
    }

    TransportControls { anchors.centerIn: parent; scale: 0.8 }

    SeekBar {
        anchors { horizontalCenter: parent.horizontalCenter; bottom: parent.bottom; bottomMargin: Theme.spaceXs }
        width: Math.max(0, Math.min(Theme.capsuleCenter, 2 * (Math.min(capsule.width / 2 - left.x - left.width, right.x - capsule.width / 2) - Theme.spaceMd)))
    }

    Row {
        id: right
        anchors { right: parent.right; rightMargin: Theme.spaceLg; verticalCenter: parent.verticalCenter }
        spacing: Theme.spaceXs

        IconButton { icon: "queue"; tip: qsTr("Queue"); onClicked: { Nav.tab = "next"; Nav.nowPlaying = true } }
        IconButton { icon: "lyrics"; tip: qsTr("Lyrics"); onClicked: { Nav.tab = "lyrics"; Nav.nowPlaying = true } }
        IconButton { icon: "sliders"; tip: qsTr("Sound"); active: Nav.audio; onClicked: Nav.audio = !Nav.audio }
        VolumeControl {
            anchors.verticalCenter: parent.verticalCenter
            showBar: capsule.width >= Theme.capsuleWide
        }
    }
}
