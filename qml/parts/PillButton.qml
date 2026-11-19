import QtQuick
import Snellar

Rectangle {
    id: pill
    property string text
    property string icon
    property bool primary: false
    property bool danger: false
    property real textLimit: Infinity
    readonly property bool hovered: area.containsMouse
    readonly property bool pressed: area.pressed
    readonly property real visualScale: lens.value
    readonly property color ink: primary ? Theme.accentInk : (danger ? Theme.white : Theme.text)
    property color tint: ink
    signal clicked()

    activeFocusOnTab: visible && enabled
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()

    implicitWidth: Theme.snap(content.width + 2 * Theme.pillPadding)
    implicitHeight: Theme.controlLg
    radius: height / 2
    opacity: enabled ? 1 : Theme.disabledOpacity
    color: primary ? Theme.accent : (danger ? Theme.danger : Theme.fillRaised)
    scale: lens.value

    PressLens {
        id: lens
        down: area.pressed
        w: pill.width
        h: pill.height
    }

    Rectangle {
        anchors.fill: parent
        radius: parent.radius
        color: pill.ink
        opacity: area.pressed ? Theme.fillPress.a : area.containsMouse ? Theme.fillHover.a : 0
        Behavior on opacity { NumberAnimation { duration: Theme.hover } }
    }

    FocusRing { target: pill; radius: pill.radius }

    Row {
        id: content
        anchors.centerIn: parent
        spacing: Theme.spaceSm

        Icon {
            visible: pill.icon.length > 0
            anchors.verticalCenter: parent.verticalCenter
            name: pill.icon
            size: Theme.iconSm
            color: pill.tint
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
        onClicked: pill.clicked()
    }
}
