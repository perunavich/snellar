pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Row {
    id: bars
    property bool active: Api.playing
    property color color: Theme.accent
    property int step: 0
    readonly property var levels: [[0.9, 0.4, 0.7], [0.5, 0.9, 0.4], [0.7, 0.6, 1.0], [0.4, 0.8, 0.6]]
    spacing: Theme.lineGap
    height: Theme.barsHeight

    Connections {
        target: Api
        enabled: bars.active && bars.visible
        function onPositionChanged() { bars.step = (bars.step + 1) % bars.levels.length }
    }

    Repeater {
        model: 3
        delegate: Rectangle {
            required property int index
            anchors.bottom: bars.bottom
            width: Theme.barWidth
            radius: Theme.barWidth / 2
            color: bars.color
            height: bars.active ? bars.height * bars.levels[bars.step][index] : Theme.barWidth
        }
    }
}
