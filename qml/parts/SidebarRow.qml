pragma ComponentBehavior: Bound
import QtQuick
import Snellar

FocusScope {
    id: row
    property string icon
    property string art
    property real indent: 0
    property string text
    property bool active: false
    property bool quiet: false
    property bool lit: false
    property int count: 0
    property bool mark: false
    property bool dot: false
    property real fold: 0
    property bool bare: false
    readonly property bool folded: fold > 0.99
    signal clicked()
    property var menu: null

    readonly property real visualScale: lens.value
    readonly property bool pressed: tap.pressed
    implicitHeight: Theme.sidebarRow
    implicitWidth: Theme.snap(glyph.width + name.implicitWidth + downloaded.width + badge.width + 4 * Theme.spaceSm + row.indent)
    scale: lens.value
    activeFocusOnTab: true
    Accessible.role: Accessible.Button
    Accessible.name: row.text

    PressLens {
        id: lens
        down: tap.pressed
        w: row.width
        h: row.height
    }

    function openMenu() { if (row.menu) row.menu.context(row, Theme.spaceMd, row.height) }

    Keys.onPressed: e => {
        const own = row.Window.activeFocusItem === row
        const menuKey = e.key === Qt.Key_Menu || (e.key === Qt.Key_F10 && (e.modifiers & Qt.ShiftModifier))
        e.accepted = own && (menuKey || e.key === Qt.Key_Return || e.key === Qt.Key_Enter)
        if (!e.accepted) return
        if (menuKey) openMenu()
        else row.clicked()
    }

    Rectangle {
        objectName: "rowFill"
        anchors.fill: parent
        radius: Theme.rRow
        color: (row.active && !row.quiet) || row.lit ? Theme.fillStrong : (hover.hovered || row.activeFocus ? Theme.fillHover : "transparent")
        Behavior on color { ColorAnimation { duration: Theme.hover } }
    }
    FocusRing { target: row; radius: Theme.rRow }

    Icon {
        id: glyph
        anchors { left: parent.left; leftMargin: Theme.spaceSm + row.indent * (1 - row.fold) + ((Theme.sidebarRow - Theme.iconMd) / 2 - Theme.spaceSm) * row.fold; verticalCenter: parent.verticalCenter }
        visible: !row.art
        name: row.icon
        size: Theme.iconMd
        color: row.active ? Theme.text : Theme.text2
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

    Item {
        id: words
        width: row.width + (Theme.sidebarWidth - Theme.railWidth) * row.fold
        height: row.height

        Text {
            id: name
            x: glyph.x + glyph.width + Theme.spaceSm
            width: downloaded.x - Theme.spaceSm - (Theme.spaceSm + row.indent + glyph.width + Theme.spaceSm)
            anchors.verticalCenter: parent.verticalCenter
            visible: !row.bare && opacity > 0
            opacity: Motion.railWords(row.fold, row.width - x - contentWidth)
            text: row.text
            color: row.active ? Theme.text : Theme.text2
            font.pixelSize: Theme.fsBody
            font.weight: row.active ? Font.Medium : Font.Normal
            elide: Text.ElideRight
        }

        Icon {
            id: downloaded
            anchors { right: badge.left; rightMargin: row.mark ? Theme.spaceXs : 0; verticalCenter: parent.verticalCenter }
            visible: row.mark && opacity > 0
            opacity: Motion.railWords(row.fold, row.width - x - width)
            width: row.mark ? Theme.iconXs : 0
            name: "downloaded"
            size: Theme.iconXs
            color: Theme.accent
        }

        Text {
            id: badge
            anchors { right: parent.right; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
            visible: row.count > 0 && opacity > 0
            opacity: Motion.railWords(row.fold, row.width - x - width)
            text: row.count > 99 ? "99+" : row.count > 0 ? row.count : ""
            color: Theme.text3
            font.pixelSize: Theme.fsCaption
            font.family: Theme.fontMono
        }
    }

    Rectangle {
        id: pip
        visible: row.dot
        x: (row.width - Theme.spaceSm - width) * (1 - row.fold) + (glyph.x + glyph.width - width / 2) * row.fold
        y: (row.height - height) / 2 * (1 - row.fold) + (glyph.y - height / 2) * row.fold
        width: Theme.dotSize
        height: width
        radius: width / 2
        color: Theme.accent
    }

    HoverHandler { id: hover }
    Tap { id: tap; onTapped: row.clicked() }
    MenuTap { menu: row.menu }

    TipLoader {
        id: nameTip
        want: row.folded && row.text.length > 0 && !Nav.nowPlaying && (hover.hovered || row.activeFocus)
        cut: tap.pressed
        hint: row.text
        sourceComponent: Tip {
            text: nameTip.hint
            want: nameTip.want
            cut: nameTip.cut
            x: Theme.navRight ? -implicitWidth - Theme.spaceMd - Theme.spaceSm : row.width + Theme.spaceMd + Theme.spaceSm
            y: (row.height - implicitHeight) / 2
        }
    }
}
