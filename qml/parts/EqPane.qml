pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: pane
    property var fx
    readonly property bool picking: menu.opened
    signal changed(string key, var value)

    readonly property var freqs: ["31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"]
    readonly property var presets: [
        { name: qsTr("Flat"), eq: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0] },
        { name: qsTr("Bass boost"), eq: [6, 5, 4, 2, 1, 0, 0, 0, 0, 0] },
        { name: qsTr("Vocal"), eq: [-2, -2, -1, 1, 3, 4, 3, 1, 0, -1] },
        { name: qsTr("Treble"), eq: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6] },
        { name: qsTr("Loudness"), eq: [5, 3, 0, -1, -1, 0, -1, 0, 3, 4] },
        { name: qsTr("Electronic"), eq: [5, 4, 1, 0, -2, 2, 1, 1, 4, 5] }
    ]
    readonly property int preset: presets.findIndex(p => JSON.stringify(p.eq) === JSON.stringify(fx.eq))
    readonly property real bandsLeft: Theme.eqScale + Theme.eqPre + Theme.spaceXl
    readonly property real column: (width - bandsLeft) / freqs.length
    readonly property real half: (Theme.eqHeight - Theme.knob) / 2

    function closeMenu() { menu.close() }

    spacing: Theme.spaceXl

    Rectangle {
        id: picker
        width: Theme.snap(pickRow.width + 2 * Theme.spaceMd)
        height: Theme.segmentHeight
        radius: Theme.rSm
        color: pickArea.containsMouse || menu.opened ? Theme.fillRaised : Theme.fillHover

        Row {
            id: pickRow
            anchors.centerIn: parent
            spacing: Theme.spaceXs
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: pane.preset >= 0 ? pane.presets[pane.preset].name : qsTr("Custom")
                color: Theme.text
                font.pixelSize: Theme.fsCallout
                font.weight: Font.Medium
            }
            Icon { anchors.verticalCenter: parent.verticalCenter; name: "down"; size: Theme.iconSm; color: Theme.text2 }
        }

        MouseArea {
            id: pickArea
            anchors.fill: parent
            hoverEnabled: true
            cursorShape: Qt.PointingHandCursor
            onClicked: menu.toggle(picker, 0, picker.height + Theme.spaceXs)
        }
    }

    RiffMenu {
        id: menu
        Instantiator {
            model: pane.presets
            delegate: MenuRow {
                required property var modelData
                required property int index
                text: modelData.name
                marked: index === pane.preset
                onTriggered: pane.changed("eq", modelData.eq)
            }
            onObjectAdded: (i, o) => menu.insertItem(i, o)
            onObjectRemoved: (i, o) => menu.removeItem(o)
        }
        MenuRow {
            text: qsTr("Custom")
            shown: pane.preset < 0
            divider: true
            marked: true
        }
    }

    Item {
        width: parent.width
        height: pre.implicitHeight

        Repeater {
            model: [12, 0, -12]
            delegate: Item {
                id: mark
                required property int modelData
                readonly property real at: Theme.eqHeight / 2 - modelData / 12 * pane.half
                width: pane.width
                height: 1
                y: at

                Text {
                    width: Theme.eqScale - Theme.spaceSm
                    anchors.verticalCenter: parent.verticalCenter
                    horizontalAlignment: Text.AlignRight
                    text: mark.modelData > 0 ? "+" + mark.modelData : mark.modelData < 0 ? "−" + -mark.modelData : "0"
                    color: Theme.text3
                    font.pixelSize: Theme.fsCaption
                    font.weight: Font.Medium
                }
                Rectangle {
                    x: Theme.eqScale
                    width: Theme.eqPre
                    height: 1
                    color: mark.modelData === 0 ? Theme.fillPress : Theme.separator
                }
                Rectangle {
                    x: pane.bandsLeft
                    width: pane.width - x
                    height: 1
                    color: mark.modelData === 0 ? Theme.fillPress : Theme.separator
                }
            }
        }

        EqBand {
            id: pre
            x: Theme.eqScale
            width: Theme.eqPre
            label: qsTr("Pre")
            value: pane.fx.preamp
            onMoved: v => pane.changed("preamp", v)
        }

        Repeater {
            model: pane.freqs
            delegate: EqBand {
                required property string modelData
                required property int index
                x: pane.bandsLeft + index * pane.column
                width: pane.column
                label: modelData
                value: pane.fx.eq[index]
                onMoved: v => pane.changed("eq", pane.fx.eq.map((g, i) => i === index ? v : g))
            }
        }
    }
}
