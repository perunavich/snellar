import QtQuick
import Snellar

Row {
    id: transport
    property bool compact: false
    readonly property real stepSize: compact ? Theme.controlSm : Theme.controlMd
    readonly property real disc: compact ? Theme.controlLg : Theme.playPill
    spacing: compact ? Theme.spaceSm : Theme.spaceMd

    ModeButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "shuffle"
        tip: qsTr("Shuffle")
        on: Api.shuffle
        reach: (Theme.controlLg - size) / 2
        onClicked: Api.toggleShuffle()
    }
    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "prev"
        tip: qsTr("Previous")
        size: transport.stepSize
        iconSize: Theme.iconMd
        reach: (Theme.controlLg - size) / 2
        color: Theme.text
        onClicked: Api.prev()
    }

    Item {
        width: transport.disc + Theme.spaceSm
        height: transport.disc
        anchors.verticalCenter: parent.verticalCenter

        Rectangle {
            id: playFace
            objectName: "playFace"
            anchors.centerIn: parent
            width: transport.disc
            height: transport.disc
            radius: height / 2 + (Theme.rLg - height / 2) * shape.value
            color: Theme.accent
            scale: lens.value

            PressLens {
                id: lens
                down: playArea.pressed
                w: playFace.width
                h: playFace.height
            }

            SpringValue {
                id: shape
                reducible: true
                zeta: Motion.controlDamping
                response: Motion.controlResponse
                target: Api.playing ? 1 : 0
            }

            Icon {
                anchors.centerIn: parent
                name: Api.loading ? "refresh" : (Api.playing ? "pause" : "play")
                size: Theme.iconLg
                color: Theme.accentInk
                spinning: Api.loading
            }

            MouseArea {
                id: playArea
                anchors.fill: parent
                hoverEnabled: true
                onClicked: Api.toggle()
            }

            TipLoader {
                want: playArea.containsMouse && !playArea.pressed
                cut: playArea.pressed
                hint: Api.playing ? qsTr("Pause") : qsTr("Play")
            }
        }
    }

    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "next"
        tip: qsTr("Next")
        size: transport.stepSize
        iconSize: Theme.iconMd
        reach: (Theme.controlLg - size) / 2
        color: Theme.text
        onClicked: Api.next()
    }
    ModeButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: Api.repeat === "one" ? "repeatOne" : "repeat"
        tip: Api.repeat === "one" ? qsTr("Repeat one") : qsTr("Repeat")
        on: Api.repeat !== "off"
        reach: (Theme.controlLg - size) / 2
        onClicked: Api.cycleRepeat()
    }
}
