import QtQuick
import Snellar

Row {
    spacing: Theme.spaceMd

    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "shuffle"
        tip: qsTr("Shuffle")
        size: Theme.controlSm
        iconSize: Theme.iconSm
        active: Api.shuffle
        onClicked: Api.call("shuffle", { on: !Api.shuffle })
    }
    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "prev"
        tip: qsTr("Previous")
        size: Theme.controlMd
        iconSize: Theme.iconMd
        color: Theme.text
        onClicked: Api.prev()
    }

    Item {
        width: Theme.playPill + Theme.spaceSm
        height: Theme.playPill
        anchors.verticalCenter: parent.verticalCenter

        Rectangle {
            anchors.centerIn: parent
            width: playArea.pressed ? Theme.playPill + Theme.spaceSm : Theme.playPill
            height: playArea.pressed ? Theme.controlLg : Theme.playPill
            radius: Api.playing ? Theme.rLg : height / 2
            color: Theme.accent
            Behavior on width { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }
            Behavior on height { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }
            Behavior on radius { NumberAnimation { duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }

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
                cursorShape: Qt.PointingHandCursor
                onClicked: Api.toggle()
            }

            Loader {
                anchors.fill: parent
                active: playArea.containsMouse && !playArea.pressed
                sourceComponent: Tip { text: Api.playing ? qsTr("Pause") : qsTr("Play") }
            }
        }
    }

    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: "next"
        tip: qsTr("Next")
        size: Theme.controlMd
        iconSize: Theme.iconMd
        color: Theme.text
        onClicked: Api.next()
    }
    IconButton {
        anchors.verticalCenter: parent.verticalCenter
        icon: Api.repeat === "one" ? "repeatOne" : "repeat"
        tip: Api.repeat === "one" ? qsTr("Repeat one") : qsTr("Repeat")
        size: Theme.controlSm
        iconSize: Theme.iconSm
        active: Api.repeat !== "off"
        onClicked: Api.call("repeat", { mode: Api.repeat === "off" ? "all" : (Api.repeat === "all" ? "one" : "off") })
    }
}
