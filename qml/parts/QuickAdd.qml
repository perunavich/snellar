pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

IconButton {
    id: button
    property var lists: []
    property var song: null
    property bool added: false
    property bool naming: false
    readonly property string playing: Api.current ? Api.current.id : ""
    readonly property bool picking: picker.opened

    visible: !!Api.current
    icon: added ? "check" : "plus"
    tip: picker.opened ? "" : qsTr("Add to playlist")
    active: added || picker.opened
    onClicked: picker.opened ? picker.close() : picker.open()
    onPlayingChanged: added = false
    onPickingChanged: Nav.picking = picking
    onVisibleChanged: if (!visible) picker.close()

    function add(list) {
        const tracks = [song]
        picker.close()
        Api.addToPlaylist(list, tracks, fresh => {
            if (!fresh) return
            added = true
            settle.restart()
        })
    }

    function create() {
        const name = field.text.trim()
        if (!name) return
        Api.call("createPlaylist", { name: name }, id => add({ id: id, name: name }))
    }

    Timer {
        id: settle
        interval: Theme.toastTime
        onTriggered: button.added = false
    }

    Popup {
        id: picker
        property bool above: false
        readonly property real tallest: (Theme.pickerRows + 1) * Theme.controlMd + 2 * Theme.spaceSm
        x: 0
        y: above ? -height - Theme.spaceXs : button.height + Theme.spaceXs
        width: Theme.menuWidth
        margins: Theme.gap
        padding: Theme.spaceSm
        focus: true
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent

        onAboutToShow: {
            button.song = Api.current
            button.naming = false
            field.clear()
            picker.above = button.mapToItem(null, 0, button.height).y + picker.tallest + Theme.gap > button.Window.height
            Api.call("recentPlaylists", {}, v => { if (JSON.stringify(v || []) !== JSON.stringify(button.lists)) button.lists = v || [] })
        }
        onOpened: places.forceActiveFocus()

        background: Rectangle {
            radius: Theme.rLg
            color: Theme.menu
            border.width: 1
            border.color: Theme.separator
        }

        contentItem: Column {
            spacing: Theme.spaceXs

            Item {
                width: parent.width
                height: Theme.controlMd

                Rectangle {
                    anchors.fill: parent
                    visible: !button.naming
                    radius: Theme.rSm
                    color: newArea.containsMouse ? Theme.fillHover : "transparent"
                }
                Icon {
                    id: newIcon
                    x: Theme.spaceMd
                    anchors.verticalCenter: parent.verticalCenter
                    visible: !button.naming
                    name: "plus"
                    size: Theme.iconSm
                    color: Theme.text
                }
                Text {
                    anchors { left: newIcon.right; leftMargin: Theme.spaceSm; right: parent.right; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
                    visible: !button.naming
                    text: qsTr("New playlist")
                    color: Theme.text
                    font.pixelSize: Theme.fsBody
                    elide: Text.ElideRight
                }
                MouseArea {
                    id: newArea
                    anchors.fill: parent
                    enabled: !button.naming
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    onClicked: {
                        button.naming = true
                        field.forceActiveFocus()
                    }
                }
                TextField {
                    id: field
                    anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter }
                    height: Theme.controlSm
                    visible: button.naming
                    leftPadding: Theme.spaceMd
                    placeholderText: qsTr("Playlist name")
                    placeholderTextColor: Theme.text3
                    color: Theme.text
                    font.pixelSize: Theme.fsCallout
                    background: Rectangle { radius: height / 2; color: Theme.fillRaised }
                    onAccepted: button.create()
                    Keys.onEscapePressed: {
                        button.naming = false
                        places.forceActiveFocus()
                    }
                }
            }

            Rectangle {
                width: parent.width
                height: 1
                visible: places.count > 0
                color: Theme.separator
            }

            ListView {
                id: places
                width: parent.width
                height: Math.min(contentHeight, Theme.pickerRows * Theme.controlMd)
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                model: button.lists
                ScrollBar.vertical: ScrollBar { }
                Keys.onReturnPressed: if (currentIndex >= 0) button.add(button.lists[currentIndex])
                Keys.onEnterPressed: if (currentIndex >= 0) button.add(button.lists[currentIndex])

                delegate: Item {
                    id: place
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    height: Theme.controlMd

                    Rectangle {
                        anchors.fill: parent
                        radius: Theme.rSm
                        color: placeArea.containsMouse || (place.ListView.isCurrentItem && places.activeFocus) ? Theme.fillHover : "transparent"
                    }
                    Text {
                        anchors { left: parent.left; right: parent.right; leftMargin: Theme.spaceMd; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
                        text: place.modelData.name
                        color: Theme.text
                        font.pixelSize: Theme.fsBody
                        elide: Text.ElideRight
                    }
                    MouseArea {
                        id: placeArea
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.PointingHandCursor
                        onClicked: button.add(place.modelData)
                    }
                }
            }
        }
    }
}
