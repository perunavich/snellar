pragma ComponentBehavior: Bound
import QtQuick
import Snellar

Item {
    id: btn
    property string icon
    property real size: Theme.controlMd
    property real iconSize: Theme.iconMd
    property color color: Theme.text2
    property color activeColor: Theme.accent
    property bool active: false
    property bool filledBg: false
    property bool spinning: false
    property string tip
    signal clicked()

    width: size
    height: size

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: btn.filledBg ? Theme.fillRaised : Theme.text
        opacity: btn.filledBg ? 1 : (area.pressed ? 0.14 : area.containsMouse ? 0.08 : 0)
        Behavior on opacity { NumberAnimation { duration: Theme.hover; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveHover } }
    }

    Icon {
        anchors.centerIn: parent
        name: btn.icon
        size: btn.iconSize
        spinning: btn.spinning
        color: btn.active ? btn.activeColor : btn.color
        scale: area.pressed ? 0.88 : 1
        Behavior on scale { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        enabled: btn.enabled
        onClicked: btn.clicked()
    }

    Loader {
        anchors.fill: parent
        active: area.containsMouse && !area.pressed && btn.tip.length > 0
        sourceComponent: Tip { text: btn.tip }
    }
}
