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
    property alias menuKind: spot.kind
    property alias menuEntity: spot.entity
    property var warms: null
    readonly property bool hovered: hover.hovered
    signal opened()
    signal played()

    width: cover
    height: cover + Theme.cardText

    HoverHandler { id: hover }
    Intent { target: card; hovered: hover.hovered; page: card.warms }
    Tap { onTapped: card.opened() }
    MenuTap { menu: spot }
    MenuSpot { id: spot }

    Cover {
        id: art
        width: card.cover
        height: card.cover
        radius: card.round ? card.cover / 2 : Theme.rArtM
        px: 400
        decode: Math.ceil(card.cover * Screen.devicePixelRatio)
        url: card.url

        Rectangle {
            visible: card.playable
            width: Theme.controlLg
            height: Theme.controlLg
            radius: Theme.controlLg / 2
            color: Theme.accent
            anchors { right: parent.right; bottom: parent.bottom; margins: Theme.spaceMd }
            opacity: hover.hovered || card.live ? 1 : 0
            transform: Scale { origin.x: Theme.controlLg / 2; origin.y: Theme.controlLg / 2; xScale: fabLens.value; yScale: fabLens.value }
            Behavior on opacity { NumberAnimation { duration: Theme.fast } }

            PressLens { id: fabLens; down: fabArea.pressed; w: Theme.controlLg; h: Theme.controlLg }

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
            wrapMode: card.round ? Text.WordWrap : Text.NoWrap
            maximumLineCount: 2
            elide: Text.ElideRight
        }
    }
}
