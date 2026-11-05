import QtQuick
import Snellar

Rectangle {
    id: pill
    property string text
    property string icon
    property bool primary: false
    property bool danger: false
    property real textLimit: Infinity
    readonly property color ink: primary ? Theme.accentInk : (danger ? Theme.white : Theme.text)
    signal clicked()

    implicitWidth: Theme.snap(content.width + 2 * Theme.pillPadding)
    implicitHeight: Theme.controlLg
    radius: height / 2
    color: primary ? Theme.accent : (danger ? Theme.red : Theme.fillRaised)
    scale: area.pressed ? 0.95 : 1
    Behavior on scale { NumberAnimation { duration: Theme.normal; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveJelly } }

    Rectangle {
        anchors.fill: parent
        radius: parent.radius
        color: pill.ink
        opacity: area.pressed ? 0.14 : area.containsMouse ? 0.08 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.hover } }
    }

    Row {
        id: content
        anchors.centerIn: parent
        spacing: Theme.spaceSm

        Icon {
            visible: pill.icon.length > 0
            anchors.verticalCenter: parent.verticalCenter
            name: pill.icon
            size: Theme.iconSm
            color: pill.ink
        }
        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: Math.min(implicitWidth, pill.textLimit)
            text: pill.text
            elide: Text.ElideRight
            color: pill.ink
            font.pixelSize: Theme.fsBody
            font.weight: Font.DemiBold
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: Qt.PointingHandCursor
        onClicked: pill.clicked()
    }
}
