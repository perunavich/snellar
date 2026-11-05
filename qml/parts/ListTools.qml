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
        hint: qsTr("Find in list")
        query: tools.sorter.query
        onEdited: text => tools.sorter.query = text
    }

    Rectangle {
        id: sortButton
        width: Theme.snap(sortRow.width + 2 * Theme.spaceMd)
        height: Theme.controlSm
        radius: height / 2
        color: sortArea.containsMouse ? Theme.fillPress : Theme.fillRaised
        Behavior on color { ColorAnimation { duration: Theme.hover } }

        Row {
            id: sortRow
            anchors.centerIn: parent
            spacing: Theme.spaceXs
            Icon { anchors.verticalCenter: parent.verticalCenter; name: "sort"; size: Theme.iconSm; color: Theme.text2 }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: tools.sorter.label
                color: Theme.text
                font.pixelSize: Theme.fsCallout
            }
            Icon { anchors.verticalCenter: parent.verticalCenter; name: tools.sorter.reversed ? "up" : "down"; size: Theme.iconSm; color: Theme.text2 }
        }

        MouseArea {
            id: sortArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: menu.popup(sortButton, 0, sortButton.height + Theme.spaceXs)
        }
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
