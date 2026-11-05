import QtQuick
import Snellar

Item {
    id: root
    property string label
    property real value: 0
    property real from: 0
    property real to: 1
    property int decimals: 0
    property string unit: ""
    property bool dragging: false
    property real dragValue: 0
    readonly property real fraction: (value - from) / (to - from)
    readonly property real shown: dragging ? dragValue : (sv.moving ? sv.value : fraction)
    signal moved(real v)

    implicitWidth: Theme.segmentWidth
    implicitHeight: Theme.segmentHeight

    SpringValue { id: sv; zeta: 0.8; response: 0.4 }

    function emit(f) { root.moved(from + f * (to - from)) }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: Theme.fillRaised
        clip: true

        Rectangle {
            width: parent.width * root.shown
            visible: width > 0.5
            height: parent.height
            radius: parent.radius
            color: Theme.accent
            opacity: 0.9
        }

        Text {
            anchors { left: parent.left; leftMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
            text: root.label
            color: root.shown > 0.2 ? Theme.accentInk : Theme.text
            font.pixelSize: Theme.fsCallout
            font.weight: Font.Medium
        }

        Text {
            anchors { right: parent.right; rightMargin: Theme.spaceMd; verticalCenter: parent.verticalCenter }
            text: (root.from + root.shown * (root.to - root.from)).toFixed(root.decimals) + root.unit
            color: root.shown > 0.92 ? Theme.accentInk : Theme.text
            font.pixelSize: Theme.fsCaption
            font.weight: Font.DemiBold
        }
    }

    MouseArea {
        anchors.fill: parent
        preventStealing: true
        cursorShape: Qt.PointingHandCursor
        property real grabDx: 0
        property real pressX: 0
        property bool didMove: false

        onPressed: m => {
            pressX = m.x
            didMove = false
            grabDx = m.x - root.shown * width
            root.dragValue = root.shown
            root.dragging = true
        }
        onPositionChanged: m => {
            if (!pressed || (!didMove && Math.abs(m.x - pressX) < 4)) return
            didMove = true
            root.dragValue = Math.max(0, Math.min(1, (m.x - grabDx) / width))
            root.emit(root.dragValue)
        }
        onReleased: finish()
        onCanceled: finish()

        function finish() {
            if (!didMove) {
                const f = Math.max(0, Math.min(1, pressX / width))
                sv.snap(root.shown)
                root.dragging = false
                sv.target = f
                root.emit(f)
            } else {
                root.dragging = false
            }
        }
    }
}
