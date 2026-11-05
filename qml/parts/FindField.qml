import QtQuick
import QtQuick.Controls
import Snellar

Rectangle {
    id: find
    property string query
    property string hint
    signal edited(string text)
    signal dismissed()

    function focusField() {
        field.forceActiveFocus()
        field.selectAll()
    }

    width: Theme.fieldWidth
    height: Theme.controlSm
    radius: height / 2
    color: field.activeFocus ? Theme.fillPress : Theme.fillRaised
    Behavior on color { ColorAnimation { duration: Theme.hover } }

    Icon {
        id: lens
        anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
        name: "search"
        size: Theme.iconSm
        color: Theme.text3
    }

    TextField {
        id: field
        anchors { left: lens.right; right: clear.left; leftMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
        background: null
        color: Theme.text
        placeholderText: find.hint
        placeholderTextColor: Theme.text3
        font.pixelSize: Theme.fsCallout
        selectByMouse: true
        text: find.query
        onTextEdited: find.edited(text)
        Keys.onEscapePressed: { find.edited(""); focus = false; find.dismissed() }
    }

    IconButton {
        id: clear
        anchors { right: parent.right; rightMargin: Theme.spaceXs; verticalCenter: parent.verticalCenter }
        icon: "close"
        tip: qsTr("Clear")
        size: Theme.sidebarRow
        iconSize: Theme.iconXs
        visible: find.query.length > 0
        onClicked: { find.edited(""); field.forceActiveFocus() }
    }
}
