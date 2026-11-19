pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Menu {
    id: menu
    property bool shown: true
    property bool divider: false
    property string glyph
    property bool nested: false
    property Item opener: null
    property bool anchored: false
    property Item closedFrom: null
    property real room: Infinity
    readonly property MenuItem parentRow: nested ? parent as MenuItem : null
    implicitWidth: {
        let widest = 0
        for (let i = 0; i < count; i++) {
            const row = itemAt(i)
            if (row && row.visible) widest = Math.max(widest, row.implicitWidth)
        }
        return Math.min(Theme.menuWidthMax, Math.max(Theme.menuWidth, Theme.snap(widest + leftPadding + rightPadding)))
    }
    height: Math.min(implicitHeight, room)
    padding: Theme.spaceSm
    margins: Theme.spaceSm
    font.pixelSize: Theme.fsBody
    closePolicy: Popup.CloseOnEscape | (anchored ? Popup.CloseOnPressOutsideParent : Popup.CloseOnPressOutside)

    function toggle(at, x, y) {
        if (visible && opener === at) return close()
        opener = at
        anchored = true
        shape.wanted = at as Pressable
        const rows = tall()
        const top = at.mapToItem(null, 0, 0).y
        const below = Overlay.overlay.height - Theme.gap - top - y
        const above = top + at.height - y - Theme.gap
        const down = rows <= below || (rows > above && below >= above)
        room = down ? below : above
        popup(at, x, down ? y : at.height - y - Math.min(rows, above))
    }

    function tall() {
        let rows = topPadding + bottomPadding
        for (let i = 0; i < count; i++) rows += itemAt(i).height
        return rows
    }

    function context(at, x, y) {
        opener = at
        anchored = false
        room = Infinity
        shape.wanted = null
        if (visible) shape.grow(null)
        popup(at, x, y)
    }

    function forget() { closedFrom = null }

    function heldBy(at) { return closedFrom === at || (visible && opener === at) }

    onVisibleChanged: if (!nested) Nav.picking = visible
    onAboutToHide: {
        closedFrom = opener
        Qt.callLater(forget)
    }
    onClosed: {
        opener = null
        anchored = false
        shape.wanted = null
    }

    Shortcut {
        sequences: ["Menu", "Shift+F10"]
        enabled: menu.opened && menu.opener !== null
        onActivated: menu.close()
    }

    Connections {
        target: menu.parentRow ? menu.parentRow.menu : null
        function onAboutToHide() { shape.rest(0) }
    }

    background: LiquidShape {
        id: shape
        popup: menu
        content: menu.contentItem
        attach: menu.parentRow
    }

    delegate: MenuRow {
        readonly property RiffMenu owner: subMenu as RiffMenu
        shown: !owner || owner.shown
        available: !owner || owner.enabled
        divider: !!owner && owner.divider
        glyph: owner ? owner.glyph : ""
    }
}
