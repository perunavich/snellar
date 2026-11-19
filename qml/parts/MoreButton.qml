import QtQuick
import Snellar

PopButton {
    id: more
    property alias kind: own.kind
    property alias entity: own.entity
    property MenuSpot menu: own
    property bool page: false
    property bool inRow: false
    property bool rowHovered: false

    icon: "more"
    tip: qsTr("More")
    size: page ? Theme.controlLg : Theme.controlSm
    iconSize: inRow ? Theme.iconSm : Theme.iconMd
    rest: page ? Theme.fillRaised : "transparent"
    opacity: !inRow || rowHovered || held ? 1 : 0
    Behavior on opacity { enabled: more.inRow; NumberAnimation { duration: Theme.hover } }
    onClicked: menu.toggle(more, 0, more.height)

    MenuSpot { id: own }
}
