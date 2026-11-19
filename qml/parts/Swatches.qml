pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Flow {
    id: swatches
    property var names: ({})
    readonly property var options: ["glass", "lagoon", "teal", "lilac", "coral", "cover"]
    readonly property int cell: Math.max(Theme.controlSm + 2 * Theme.spaceSm, Theme.snap(options.reduce((widest, o) => Math.max(widest, metrics.advanceWidth(names[o] || "")), 0)))
    signal picked(string name)
    spacing: Theme.spaceSm

    FontMetrics {
        id: metrics
        font.pixelSize: Theme.fsCaption
        font.weight: Font.Medium
    }

    Repeater {
        model: swatches.options
        delegate: Item {
            id: option
            required property string modelData
            readonly property bool chosen: Theme.accentName === modelData
            readonly property bool fromCover: modelData === "cover"
            width: swatches.cell
            height: dot.height + Theme.spaceXs + name.implicitHeight

            Item {
                id: dot
                anchors.horizontalCenter: parent.horizontalCenter
                width: Theme.controlSm
                height: Theme.controlSm
                scale: lens.value

                PressLens { id: lens; down: area.pressed; w: dot.width; h: dot.height }

                Rectangle {
                    anchors.fill: parent
                    radius: width / 2
                    color: "transparent"
                    border.width: Theme.ring
                    border.color: option.chosen ? Theme.text : area.containsMouse ? Theme.fillStrong : "transparent"
                    Behavior on border.color { ColorAnimation { duration: Theme.hover } }
                }
                Rectangle {
                    anchors.centerIn: parent
                    width: parent.width - Theme.spaceSm
                    height: width
                    radius: width / 2
                    visible: !option.fromCover
                    color: option.fromCover ? "transparent" : Theme.palettes[option.modelData].accent
                }
                Cover {
                    anchors.centerIn: parent
                    width: parent.width - Theme.spaceSm
                    height: width
                    radius: width / 2
                    visible: option.fromCover
                    px: 64
                    url: Api.current ? Api.current.thumb : ""
                }
                Rectangle {
                    anchors.centerIn: parent
                    width: parent.width - Theme.spaceSm
                    height: width
                    radius: width / 2
                    visible: option.fromCover && option.chosen
                    color: Theme.coverShade
                }
                Icon {
                    anchors.centerIn: parent
                    visible: option.chosen
                    name: "check"
                    size: Theme.iconSm
                    color: option.fromCover ? Theme.white : Theme.palettes[option.modelData].ink
                }
            }
            Text {
                id: name
                anchors { top: dot.bottom; topMargin: Theme.spaceXs; horizontalCenter: parent.horizontalCenter }
                text: swatches.names[option.modelData] || ""
                color: option.chosen ? Theme.text : Theme.text2
                font.pixelSize: Theme.fsCaption
                font.weight: Font.Medium
            }
            MouseArea { id: area; anchors.fill: parent; hoverEnabled: true; onClicked: swatches.picked(option.modelData) }
        }
    }
}
