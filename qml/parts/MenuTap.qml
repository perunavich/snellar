import QtQuick

TapHandler {
    id: tap
    property var menu: null
    property bool closing: false
    signal opening()

    enabled: !!menu
    acceptedButtons: Qt.RightButton
    onPressedChanged: if (pressed) closing = menu.heldBy(tap.parent)
    onTapped: p => {
        if (closing) return menu.close()
        tap.opening()
        menu.context(tap.parent, p.position.x, p.position.y)
    }
}
