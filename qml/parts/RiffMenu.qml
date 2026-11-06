pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Menu {
    id: menu
    property bool shown: true
    property bool divider: false
    property Item opener: null
    property bool anchored: false
    property Item closedFrom: null
    implicitWidth: Theme.menuWidth
    padding: Theme.spaceSm
    font.pixelSize: Theme.fsBody
    closePolicy: Popup.CloseOnEscape | (anchored ? Popup.CloseOnPressOutsideParent : Popup.CloseOnPressOutside)

    function toggle(at, x, y) {
        if (visible && opener === at) return close()
        opener = at
        anchored = true
        popup(at, x, y)
    }

    function context(at, x, y) {
        opener = at
        anchored = false
        popup(at, x, y)
    }

    function forget() { closedFrom = null }

    function heldBy(at) { return closedFrom === at || (visible && opener === at) }

    onAboutToHide: {
        closedFrom = opener
        Qt.callLater(forget)
    }
    onClosed: {
        opener = null
        anchored = false
    }

    Shortcut {
        sequences: ["Menu", "Shift+F10"]
        enabled: menu.opened && menu.opener !== null
        onActivated: menu.close()
    }

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
