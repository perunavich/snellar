pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Snellar

Popup {
    id: help
    property var shortcuts: []
    readonly property var groups: shortcuts.reduce((all, s) => all.indexOf(s.group) >= 0 ? all : all.concat([s.group]), [])

    function shown(sequence) {
        const names = { Left: "←", Right: "→", Up: "↑", Down: "↓", Space: qsTr("Space"), Escape: "Esc", Return: "Enter", Menu: qsTr("Menu key") }
        return sequence.split("+").map(k => names[k] || k).join(" + ")
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(Theme.keyHelpWidth, parent.width - 2 * Theme.space2xl)
    height: Math.min(implicitHeight, parent.height - 2 * Theme.space2xl)
    modal: true
    focus: true
    padding: Theme.spaceXl

    Overlay.modal: Rectangle { color: Theme.scrim }

    Connections {
        target: Nav
        function onKeysChanged() { if (Nav.keys) help.open(); else help.close() }
    }
    onClosed: Nav.keys = false

    Shortcut {
        sequence: "Escape"
        enabled: help.opened
        onActivated: help.close()
    }

    background: Rectangle {
        radius: Theme.rLg
        color: Theme.menu
        border.width: 1
        border.color: Theme.separator
    }

    contentItem: Flickable {
        implicitHeight: body.height
        contentHeight: body.height
        clip: true
        boundsBehavior: Flickable.StopAtBounds

        Column {
            id: body
            width: parent.width
            spacing: Theme.spaceLg

            Item {
                width: parent.width
                height: Theme.controlMd

                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: qsTr("Keyboard shortcuts")
                    color: Theme.text
                    font.pixelSize: Theme.fsTitle2
                    font.weight: Font.DemiBold
                }
                IconButton {
                    anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                    icon: "close"
                    tip: qsTr("Close")
                    onClicked: help.close()
                }
            }

            Flow {
                width: parent.width
                spacing: Theme.spaceXl

                Repeater {
                    model: help.groups

                    delegate: Column {
                        id: group
                        required property string modelData
                        width: (body.width - Theme.spaceXl) / 2
                        spacing: Theme.spaceXs

                        Text {
                            text: group.modelData
                            color: Theme.text3
                            font.pixelSize: Theme.fsCaption
                            font.weight: Font.DemiBold
                        }

                        Repeater {
                            model: help.shortcuts.filter(s => s.group === group.modelData)

                            delegate: Item {
                                id: entry
                                required property var modelData
                                width: group.width
                                height: Theme.controlSm

                                Text {
                                    anchors { left: parent.left; right: caps.left; rightMargin: Theme.spaceSm; verticalCenter: parent.verticalCenter }
                                    text: entry.modelData.label
                                    color: Theme.text
                                    font.pixelSize: Theme.fsCallout
                                    elide: Text.ElideRight
                                }
                                Row {
                                    id: caps
                                    anchors { right: parent.right; verticalCenter: parent.verticalCenter }
                                    spacing: Theme.spaceXs

                                    Repeater {
                                        model: entry.modelData.keys

                                        delegate: Rectangle {
                                            id: cap
                                            required property string modelData
                                            width: capText.implicitWidth + 2 * Theme.spaceSm
                                            height: Theme.keyCap
                                            radius: Theme.rSm
                                            color: Theme.fillRaised
                                            border.width: 1
                                            border.color: Theme.separator

                                            Text {
                                                id: capText
                                                anchors.centerIn: parent
                                                text: help.shown(cap.modelData)
                                                color: Theme.text2
                                                font.family: Theme.fontMono
                                                font.pixelSize: Theme.fsMicro
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
