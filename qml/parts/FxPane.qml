pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Column {
    id: pane
    property var fx
    signal changed(string key, var value)

    function db(v) { return qsTr("%1 dB").arg((v > 0 ? "+" : v < 0 ? "−" : "") + Math.abs(v).toFixed(v % 1 ? 1 : 0)) }
    function times(v) { return Number(v.toFixed(2)) + "×" }

    FxRow { text: qsTr("Bass"); key: "bass"; from: -10; to: 10; neutral: 0; step: 0.5; format: "db" }
    FxRow { text: qsTr("Speed"); key: "speed"; from: 0.5; to: 2; neutral: 1; format: "times" }
    FxSwitch { text: qsTr("Keep pitch"); key: "pitchLock" }
    FxRow { text: qsTr("Reverb"); key: "reverb" }
    FxRow { text: qsTr("Echo"); key: "echo" }
    FxRow { text: qsTr("Stereo width"); key: "stereo"; to: 2; neutral: 1; format: "times" }
    FxSwitch { text: qsTr("Normalize volume"); key: "normalize"; last: true }

    component FxRow: Item {
        id: row
        property string text
        property string key
        property alias from: slider.from
        property alias to: slider.to
        property alias neutral: slider.neutral
        property alias step: slider.step
        readonly property real value: pane.fx[key]
        property string format
        readonly property string shown: format === "db" ? pane.db(value) : format === "times" ? pane.times(value) : value.toFixed(2)
        width: pane.width
        height: Theme.controlLg

        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: Theme.fxLabel - Theme.spaceMd
            text: row.text
            color: Theme.text
            font.pixelSize: Theme.fsBody
            elide: Text.ElideRight
        }
        FxSlider {
            id: slider
            x: Theme.fxLabel
            width: parent.width - Theme.fxLabel - Theme.fxValue
            height: parent.height
            value: row.value
            onMoved: v => pane.changed(row.key, v)
        }
        Text {
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            text: row.shown
            color: slider.changed ? Theme.text : Theme.text3
            font.family: Theme.fontMono
            font.pixelSize: Theme.fsCaption
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.separator }
    }

    component FxSwitch: Item {
        id: sw
        property string text
        property string key
        property bool last: false
        width: pane.width
        height: Theme.controlLg
        Text { anchors.verticalCenter: parent.verticalCenter; text: sw.text; color: Theme.text; font.pixelSize: Theme.fsBody }
        IosSwitch {
            anchors { right: parent.right; verticalCenter: parent.verticalCenter }
            on: pane.fx[sw.key]
            onToggled: pane.changed(sw.key, !pane.fx[sw.key])
        }
        Rectangle { visible: !sw.last; anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.separator }
    }
}
