import QtQuick
import Snellar

FocusScope {
    id: row
    property string icon
    property string art
    property real indent: 0
    property string text
    property bool active: false
    property bool lit: false
    property int count: 0
    signal clicked()
    property var menu: null

    implicitHeight: Theme.sidebarRow
    implicitWidth: Theme.snap(glyph.width + name.implicitWidth + badge.width + 4 * Theme.spaceSm + row.indent)

    Rectangle {
        anchors.fill: parent
        radius: Theme.rRow
        color: row.active || row.lit ? Theme.accentFill : (hover.hovered ? Theme.fillHover : "transparent")
        Behavior on color { ColorAnimation { duration: Theme.hover } }
    }

    Icon {
        id: glyph
        anchors { left: parent.left; leftMargin: Theme.spaceSm + row.indent; verticalCenter: parent.verticalCenter }
        visible: !row.art
        name: row.icon
        size: Theme.iconSm
        color: row.active ? Theme.accent : Theme.text2
    }

    Cover {
        anchors.centerIn: glyph
        visible: !!row.art
        width: Theme.sidebarArt
        height: Theme.sidebarArt
        radius: Theme.rArtS
        px: 64
        url: row.art
    }

    Text {
        id: name
        anchors { left: glyph.right; leftMargin: Theme.spaceSm; right: badge.left; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
        text: row.text
        color: row.active ? Theme.text : Theme.text2
        font.pixelSize: Theme.fsCallout
        font.weight: row.active ? Font.Medium : Font.Normal
        elide: Text.ElideRight
    }

    Text {
        id: badge
        anchors { right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
        visible: row.count > 0
        text: row.count > 99 ? "99+" : row.count
        color: Theme.text3
        font.pixelSize: Theme.fsCaption
        font.family: Theme.fontMono
    }

    HoverHandler { id: hover; cursorShape: Qt.PointingHandCursor }
    Tap { onTapped: row.clicked() }
    MenuTap { menu: row.menu }
}
