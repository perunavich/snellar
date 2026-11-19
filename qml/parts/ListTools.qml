pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Row {
    id: tools
    required property Sorter sorter
    spacing: Theme.spaceSm

    function focusField() { field.focusField() }

    Component.onCompleted: Nav.finder = tools
    Component.onDestruction: if (Nav.finder === tools) Nav.finder = null

    FindField {
        id: field
        height: Theme.controlLg
        hint: qsTr("Find in list")
        query: tools.sorter.query
        onEdited: text => tools.sorter.query = text
    }

    PopPill {
        id: sortButton
        height: Theme.controlLg
        lead: "sort"
        text: tools.sorter.label
        tail: tools.sorter.reversed ? "up" : "down"
        onClicked: menu.toggle(sortButton, 0, sortButton.height + Theme.spaceXs)
    }

    RiffMenu {
        id: menu
        Instantiator {
            model: tools.sorter.options
            delegate: MenuRow {
                required property var modelData
                text: modelData.label
                marked: modelData.key === tools.sorter.key
                onTriggered: tools.sorter.pick(modelData.key)
            }
            onObjectAdded: (i, o) => menu.insertItem(i, o)
            onObjectRemoved: (i, o) => menu.removeItem(o)
        }
    }
}
