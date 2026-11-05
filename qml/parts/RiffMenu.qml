pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Menu {
    id: menu
    property bool shown: true
    property bool divider: false
    implicitWidth: Theme.menuWidth
    padding: Theme.spaceSm
    font.pixelSize: Theme.fsBody

    background: Rectangle {
        radius: Theme.rLg
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator
    }

    delegate: MenuRow {
        readonly property RiffMenu owner: subMenu as RiffMenu
        shown: !owner || owner.shown
        available: !owner || owner.enabled
        divider: !!owner && owner.divider
    }
}
