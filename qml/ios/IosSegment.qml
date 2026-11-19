pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: root
    property var options: []
    property string current: ""
    property var peers: []
    signal picked(string key)

    implicitWidth: Math.max(Theme.segmentWidth, fit)
    implicitHeight: Theme.segmentHeight

    readonly property int index: {
        for (let i = 0; i < options.length; i++)
            if (options[i].key === current) return i
        return 0
    }
    readonly property real fit: options.concat(peers).reduce((widest, o) => Math.max(widest, Math.ceil(metrics.advanceWidth(o.label)) + 2 * Theme.spaceSm), 0) * options.length + 2 * Theme.segmentInset
    readonly property real seg: options.length > 0 ? (width - 2 * Theme.segmentInset) / options.length : width

    FontMetrics {
        id: metrics
        font.pixelSize: Theme.fsCallout
        font.weight: Font.DemiBold
    }

    Rectangle {
        anchors.fill: parent
        radius: height / 2
        color: Theme.fillHover

        Rectangle {
            width: root.seg
            height: parent.height - 2 * Theme.segmentInset
            y: Theme.segmentInset
            x: Theme.segmentInset + root.seg * slide.value
            radius: height / 2
            color: Theme.text
            visible: root.options.length > 0

            SpringValue {
                id: slide
                reducible: true
                zeta: Motion.segmentDamping
                response: Motion.controlResponse
                target: root.index
            }
        }

        FocusRing { target: root; radius: height / 2 }

        Row {
            anchors.fill: parent
            anchors.margins: Theme.segmentInset

            Repeater {
                model: root.options

                Item {
                    id: cell
                    required property var modelData
                    required property int index
                    width: root.seg
                    height: parent.height

                    Rectangle {
                        anchors.fill: parent
                        radius: height / 2
                        color: Theme.fillHover
                        opacity: cellArea.containsMouse && cell.index !== root.index ? 1 : 0
                        Behavior on opacity { NumberAnimation { duration: Theme.hover } }
                    }

                    Text {
                        anchors.centerIn: parent
                        text: parent.modelData.label
                        font.pixelSize: Theme.fsCallout
                        font.weight: parent.index === root.index ? Font.DemiBold : Font.Medium
                        color: parent.index === root.index ? Theme.ink : Theme.text2
                        Behavior on color { ColorAnimation { duration: Theme.fast } }
                    }

                    MouseArea {
                        id: cellArea
                        anchors.fill: parent
                        hoverEnabled: true
                        onClicked: root.picked(parent.modelData.key)
                    }
                }
            }
        }
    }
}
