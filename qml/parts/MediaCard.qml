import QtQuick
import Snellar

Item {
    id: card
    property string url
    property string title
    property string subtitle
    property bool round: false
    property bool playable: true
    property bool live: false
    property int cover: Theme.cardCover
    property alias menuKind: menu.kind
    property alias menuEntity: menu.entity
    readonly property bool hovered: hover.hovered
    signal opened()
    signal played()

    width: cover
    height: cover + Theme.cardText

    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    TapHandler { onTapped: card.opened() }
    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: p => menu.open(card, p.position.x, p.position.y)
    }
    MenuSpot { id: menu }

    Cover {
        id: art
        width: card.cover
        height: card.cover
        radius: card.round ? card.cover / 2 : Theme.rMd
        px: 400
        url: card.url
        scale: hover.hovered ? 1.03 : 1
        Behavior on scale { NumberAnimation { duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard } }

        Rectangle {
            visible: card.playable
            width: Theme.controlLg
            height: Theme.controlLg
            radius: Theme.controlLg / 2
            color: Theme.accent
            anchors { right: parent.right; bottom: parent.bottom; margins: Theme.spaceMd }
            opacity: hover.hovered || card.live ? 1 : 0
            scale: fabArea.pressed ? 0.9 : (hover.hovered || card.live ? 1 : 0.8)
            Behavior on opacity { NumberAnimation { duration: Theme.fast } }
            Behavior on scale { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }

            Icon { anchors.centerIn: parent; name: card.live && Api.playing ? "pause" : "play"; size: Theme.iconMd; color: Theme.accentInk }
            MouseArea { id: fabArea; anchors.fill: parent; onClicked: card.live ? Api.toggle() : card.played() }
        }
    }

    Column {
        anchors { top: art.bottom; topMargin: Theme.spaceMd; left: parent.left; right: parent.right }
        spacing: Theme.lineGap

        Text {
            width: parent.width
            horizontalAlignment: card.round ? Text.AlignHCenter : Text.AlignLeft
            text: card.title
            color: Theme.text
            font.pixelSize: Theme.fsBody
            font.weight: Font.Medium
            elide: Text.ElideRight
        }
        Text {
            width: parent.width
            horizontalAlignment: card.round ? Text.AlignHCenter : Text.AlignLeft
            text: card.subtitle
            color: Theme.text2
            font.pixelSize: Theme.fsCallout
            elide: Text.ElideRight
        }
    }
}
