pragma ComponentBehavior: Bound
import QtQuick
import Snellar

FocusScope {
    id: row
    property string kind: "playlist"
    property var entity
    property string title
    property string subtitle
    property string url
    property string glyph
    readonly property var kept: entity && kind !== "songs" ? Api.collection(kind, entity.id) : null
    readonly property string phase: Api.collectionPhase(kept)
    readonly property bool hovered: hover.hovered
    signal opened()

    height: Theme.rowHeight
    activeFocusOnTab: true
    Keys.onReturnPressed: opened()
    Keys.onEnterPressed: opened()
    Keys.onMenuPressed: spot.context(row, Theme.spaceMd, height)

    Rectangle {
        anchors.fill: parent
        radius: Theme.rRow
        color: row.hovered || row.activeFocus ? Theme.fillHover : "transparent"
        Behavior on color { ColorAnimation { duration: Theme.hover } }
    }
    FocusRing { target: row; radius: Theme.rRow }

    HoverHandler { id: hover }
    TapHandler {
        gesturePolicy: TapHandler.ReleaseWithinBounds
        onTapped: {
            row.forceActiveFocus(Qt.MouseFocusReason)
            row.opened()
        }
    }
    MenuTap { menu: row.kind === "songs" ? null : spot }

    Item {
        id: lead
        anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        width: Theme.rowCover
        height: Theme.rowCover

        Cover {
            anchors.fill: parent
            visible: !row.glyph
            radius: Theme.rArtS
            px: 96
            url: row.url
        }
        Rectangle {
            anchors.fill: parent
            visible: !!row.glyph
            radius: Theme.rArtS
            color: Theme.fillRaised
            Icon { anchors.centerIn: parent; name: row.glyph; size: Theme.iconMd; color: Theme.accent }
        }
    }

    Column {
        anchors { left: lead.right; leftMargin: Theme.spaceMd; right: more.left; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        spacing: Theme.lineGap

        Text {
            width: parent.width
            text: row.title
            color: Theme.text
            font.pixelSize: Theme.fsBody
            font.weight: Font.Medium
            elide: Text.ElideRight
        }
        Row {
            width: parent.width
            spacing: Theme.spaceXs

            Icon {
                id: mark
                anchors.verticalCenter: parent.verticalCenter
                visible: row.phase === "done"
                name: "downloaded"
                size: Theme.iconXs
                color: Theme.accent
            }
            ProgressRing {
                id: ring
                anchors.verticalCenter: parent.verticalCenter
                visible: row.phase !== "off" && row.phase !== "done"
                width: Theme.iconXs
                height: Theme.iconXs
                value: row.kept && row.kept.total > 0 ? row.kept.done / row.kept.total : 0
                color: row.phase === "downloading" ? Theme.accent : Theme.text3
            }
            Text {
                width: parent.width - (mark.visible || ring.visible ? Theme.iconXs + parent.spacing : 0)
                text: row.subtitle
                color: Theme.text2
                font.pixelSize: Theme.fsCallout
                elide: Text.ElideRight
            }
        }
    }

    MoreButton {
        id: more
        anchors { right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
        visible: row.kind !== "songs"
        width: visible ? Theme.controlSm : 0
        inRow: true
        rowHovered: row.hovered
        menu: spot
    }

    MenuSpot {
        id: spot
        kind: row.kind
        entity: row.kind === "songs" ? null : row.entity
    }
}
