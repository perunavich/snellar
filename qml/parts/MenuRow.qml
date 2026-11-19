import QtQuick
import QtQuick.Controls
import Snellar

MenuItem {
    id: item
    property bool shown: true
    property bool available: true
    property bool divider: false
    property bool marked: false
    property string glyph
    readonly property real gap: divider ? Theme.spaceSm + 1 : 0

    visible: shown
    enabled: shown && available
    height: shown ? implicitHeight : 0
    implicitHeight: Theme.controlMd + gap
    topPadding: gap
    leftPadding: Theme.spaceMd
    rightPadding: marked || subMenu ? Theme.iconSm + 2 * Theme.spaceSm : Theme.spaceMd
    font: item.menu ? item.menu.font : Qt.font({ pixelSize: Theme.fsBody })

    contentItem: Item {
        implicitWidth: (mark.visible ? mark.width + Theme.spaceSm : 0) + words.implicitWidth
        Icon {
            id: mark
            anchors.verticalCenter: parent.verticalCenter
            visible: item.glyph.length > 0
            name: item.glyph
            size: Theme.iconSm
            color: words.color
        }
        Text {
            id: words
            anchors { left: parent.left; leftMargin: mark.visible ? mark.width + Theme.spaceSm : 0; right: parent.right; verticalCenter: parent.verticalCenter }
            text: item.text
            color: item.enabled ? (item.marked ? Theme.accent : Theme.text) : Theme.text3
            font: item.font
            elide: Text.ElideRight
        }
    }

    indicator: Icon {
        x: item.width - width - Theme.spaceSm
        y: item.gap + (item.height - item.gap - height) / 2
        visible: item.marked
        name: "check"
        size: Theme.iconSm
        color: Theme.accent
    }

    arrow: Icon {
        x: item.width - width - Theme.spaceSm
        y: item.gap + (item.height - item.gap - height) / 2
        visible: !!item.subMenu
        name: "forward"
        size: Theme.iconSm
        color: Theme.text2
    }

    background: Item {
        Rectangle {
            visible: item.divider
            y: Theme.spaceXs
            width: parent.width
            height: 1
            color: Theme.separator
        }
        Rectangle {
            anchors { fill: parent; topMargin: item.gap }
            radius: Theme.rSm
            color: item.highlighted ? Theme.fillHover : "transparent"
        }
    }
}
