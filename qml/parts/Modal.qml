import QtQuick
import QtQuick.Controls
import Snellar

Popup {
    id: sheet
    readonly property LiquidShape shape: surface

    function openFrom(item) {
        surface.wanted = item as Pressable
        surface.origin = item
        open()
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    modal: true
    focus: true
    padding: Theme.spaceXl

    Overlay.modal: Item {}

    background: LiquidShape {
        id: surface
        popup: sheet
        content: sheet.contentItem
        modal: true
    }
}
