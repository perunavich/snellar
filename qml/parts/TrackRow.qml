pragma ComponentBehavior: Bound
import QtQuick
import Snellar

FocusScope {
    id: row
    property var track
    property var list: []
    property int rank: 0
    property bool numbered: false
    property bool showAlbum: true
    property bool removable: false
    property bool inQueue: false
    property bool radio: false
    property bool dated: false
    property bool selected: false
    property var group: []
    property var playlistId: null
    property string from: ""
    property var origin: null
    signal removeRequested()
    signal pickRequested(int modifiers)

    readonly property bool current: !!Api.current && !!track && Api.current.id === track.id
    readonly property color secondary: selected ? Theme.text : Theme.text2
    readonly property bool live: inQueue ? rank === Api.cursor && current : current && Api.playingFrom(from)
    readonly property bool hovered: hover.hovered
    readonly property bool compact: width < 420

    height: Theme.rowHeight
    activeFocusOnTab: true

    function play() {
        if (live) { if (!Api.playing) Api.toggle() }
        else if (inQueue) Api.call("jump", { index: rank, id: track.id })
        else if (radio) Api.radio(track)
        else Api.play(list, rank, from, origin)
    }
    function toggle() {
        if (live) Api.toggle()
        else play()
    }
    function openMenu(x, y) { menu.open(row, x, y) }

    Keys.onReturnPressed: play()
    Keys.onEnterPressed: play()
    Keys.onMenuPressed: openMenu(Theme.spaceMd, height)
    Keys.onPressed: e => {
        if (e.key === Qt.Key_F10 && (e.modifiers & Qt.ShiftModifier)) {
            openMenu(Theme.spaceMd, height)
            e.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.rMd
        color: row.selected ? Theme.fillStrong : row.current ? Theme.accentFill : row.hovered || row.activeFocus ? Theme.fillGroup : "transparent"
        Behavior on color { ColorAnimation { duration: Theme.hover } }
    }

    HoverHandler { id: hover }
    DragHandler {
        target: null
        enabled: !row.inQueue
        grabPermissions: PointerHandler.CanTakeOverFromAnything
        onActiveChanged: {
            if (active) Nav.carried.begin(row.group.length > 1 ? row.group : [row.track], centroid.scenePosition)
            else Nav.carried.finish()
        }
        onCentroidChanged: if (active) Nav.carried.place(centroid.scenePosition)
    }
    Timer { interval: Theme.debounce; running: row.hovered; onTriggered: Api.prefetch(row.track) }
    TapHandler {
        id: tap
        onTapped: (p, button) => {
            row.forceActiveFocus()
            if (tapCount === 1) row.pickRequested(tap.point.modifiers)
            if (tapCount === (row.inQueue ? 1 : 2)) row.play()
        }
    }
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: p => {
            if (!row.selected) row.pickRequested(Qt.NoModifier)
            row.openMenu(p.position.x, p.position.y)
        }
    }

    Item {
        id: lead
        anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        width: Theme.rowCover
        height: Theme.rowCover

        Text {
            visible: row.numbered && !row.current && !row.hovered
            anchors.centerIn: parent
            text: row.rank + 1
            color: row.secondary
            font.pixelSize: Theme.fsCallout
            font.family: Theme.fontMono
        }

        Cover {
            visible: !row.numbered
            anchors.fill: parent
            radius: Theme.rArtS
            px: 96
            url: row.track ? row.track.thumb : ""
        }

        Rectangle {
            anchors.fill: parent
            radius: Theme.rSm
            visible: !row.numbered && (row.current || row.hovered)
            color: Theme.coverShade
        }

        PlayingBars {
            anchors.centerIn: parent
            visible: row.current && !row.hovered
        }

        Icon {
            anchors.centerIn: parent
            visible: row.hovered
            name: row.live && Api.playing ? "pause" : "play"
            size: Theme.iconMd
            color: Theme.text
        }

        MouseArea {
            anchors.fill: parent
            enabled: row.hovered
            cursorShape: Qt.PointingHandCursor
            onClicked: row.toggle()
        }
    }

    Column {
        anchors {
            left: lead.right
            leftMargin: Theme.spaceMd
            right: albumLink.left
            rightMargin: Theme.spaceLg
            verticalCenter: parent.verticalCenter
        }
        spacing: Theme.lineGap

        Row {
            id: titleLine
            spacing: Theme.spaceSm
            width: parent.width

            Text {
                width: Math.min(implicitWidth, parent.width - (explicit.visible ? explicit.width + titleLine.spacing : 0))
                text: row.track ? row.track.title : ""
                color: row.current && !row.selected ? Theme.accent : Theme.text
                font.pixelSize: Theme.fsBody
                font.weight: Font.Medium
                elide: Text.ElideRight
            }
            Rectangle {
                id: explicit
                visible: !!row.track && row.track.explicit
                anchors.verticalCenter: parent.verticalCenter
                width: Theme.iconXs
                height: Theme.iconXs
                radius: Theme.spaceXs
                color: Theme.separator
                Text { anchors.centerIn: parent; text: "E"; color: Theme.text2; font.pixelSize: Theme.fsMicro; font.weight: Font.Bold }
            }
        }

        ArtistLinks {
            width: parent.width
            artists: row.track ? row.track.artists : []
            color: row.secondary
        }
    }

    Link {
        id: albumLink
        anchors { right: when.left; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        width: row.showAlbum ? Math.max(0, Math.min(implicitWidth, row.width * 0.24)) : 0
        visible: row.showAlbum && !!row.track && !!row.track.album && row.width > 640
        text: visible ? row.track.album.name : ""
        color: row.secondary
        onClicked: Nav.album(row.track.album.id)
    }

    Text {
        id: when
        anchors { right: likeButton.left; rightMargin: visible ? Theme.spaceMd : 0; verticalCenter: parent.verticalCenter }
        visible: row.dated && row.width > 760 && text.length > 0
        width: visible ? Theme.dateColumn : 0
        text: row.dated && row.track ? Theme.since(row.track.at) : ""
        color: row.secondary
        font.pixelSize: Theme.fsCallout
        elide: Text.ElideRight
    }

    IconButton {
        id: likeButton
        anchors { right: time.left; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }
        size: row.compact ? 0 : Theme.controlSm
        visible: !row.compact
        iconSize: Theme.iconSm
        readonly property bool liked: Api.isLiked(row.track)
        icon: liked ? "heartFill" : "heart"
        tip: liked ? qsTr("Unlike") : qsTr("Like")
        active: liked
        opacity: liked || row.hovered ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.hover } }
        onClicked: Api.like(row.track)
    }

    Text {
        id: time
        anchors { right: row.compact ? parent.right : more.left; rightMargin: row.compact ? Theme.spaceMd : Theme.spaceXs; verticalCenter: parent.verticalCenter }
        width: Theme.timeColumn
        opacity: row.compact && row.hovered ? 0 : 1
        horizontalAlignment: Text.AlignRight
        text: row.track && row.track.duration > 0 ? Theme.time(row.track.duration) : ""
        color: row.secondary
        font.pixelSize: Theme.fsCallout
        font.family: Theme.fontMono
    }

    IconButton {
        id: more
        anchors { right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
        size: Theme.controlSm
        iconSize: Theme.iconSm
        icon: row.removable ? "close" : "more"
        tip: row.removable ? qsTr("Remove") : qsTr("More")
        opacity: row.hovered ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.hover } }
        onClicked: row.removable ? row.removeRequested() : row.openMenu(more.x, more.y + more.height)
    }

    MenuSpot {
        id: menu
        kind: row.group.length > 1 ? "tracks" : "track"
        entity: row.group.length > 1 ? row.group : row.track
        queueIndex: row.inQueue ? row.rank : -1
        playlistId: row.playlistId
    }
}
