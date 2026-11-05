pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: pane
    property var fx
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

    spacing: Theme.spaceXl

    Flow {
        width: parent.width
        spacing: Theme.spaceSm
        Repeater {
            model: pane.presets
            delegate: Rectangle {
                id: chip
                required property var modelData
                readonly property bool on: JSON.stringify(modelData.eq) === JSON.stringify(pane.fx.eq)
                width: Theme.snap(label.implicitWidth + 2 * Theme.spaceMd)
                height: Theme.segmentHeight
                radius: height / 2
                color: on ? Theme.accent : Theme.fillHover
                Behavior on color { ColorAnimation { duration: Theme.fast } }
                Text {
                    id: label
                    anchors.centerIn: parent
                    text: chip.modelData.name
                    color: chip.on ? Theme.accentInk : Theme.text
                    font.pixelSize: Theme.fsCallout
                    font.weight: Font.Medium
                }
                MouseArea { anchors.fill: parent; cursorShape: Qt.PointingHandCursor; onClicked: pane.changed("eq", chip.modelData.eq) }
            }
        }
    }

    Row {
        anchors.horizontalCenter: parent.horizontalCenter
        spacing: Theme.spaceMd

        EqBand {
            label: qsTr("Pre")
            value: pane.fx.preamp
            onMoved: v => pane.changed("preamp", v)
        }
        Rectangle { width: 1; height: parent.height - Theme.spaceXl; color: Theme.separator }
        Repeater {
            model: pane.freqs
            delegate: EqBand {
                required property string modelData
                required property int index
                label: modelData
                value: pane.fx.eq[index]
                onMoved: v => pane.changed("eq", pane.fx.eq.map((g, i) => i === index ? v : g))
            }
        }
    }
}
