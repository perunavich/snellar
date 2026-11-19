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
    property bool backed: false
    property bool filledBg: false
    property bool spinning: false
    property real reach: 0
    property string tip
    readonly property bool pressed: area.pressed
    readonly property real visualScale: lens.value
    signal clicked()

    width: size
    height: size
    opacity: enabled ? 1 : Theme.disabledOpacity
    activeFocusOnTab: visible && enabled
    Keys.onReturnPressed: clicked()
    Keys.onEnterPressed: clicked()

    PressLens {
        id: lens
        down: area.pressed
        w: btn.width
        h: btn.height
    }

    Item {
        anchors.fill: parent
        scale: lens.value

        Rectangle {
            objectName: "backing"
            anchors.fill: parent
            radius: width / 2
            color: Theme.fillStrong
            opacity: btn.backed ? 1 : 0
            Behavior on opacity { Ease { duration: Theme.hover; curve: Theme.curveHover } }
        }

        Rectangle {
            anchors.fill: parent
            radius: width / 2
            visible: btn.filledBg
            color: Theme.fillRaised
        }

        Rectangle {
            objectName: "wash"
            anchors.fill: parent
            radius: width / 2
            color: Theme.text
            opacity: area.pressed ? Theme.fillPress.a : area.containsMouse ? Theme.fillHover.a : 0
            Behavior on opacity { Ease { duration: Theme.hover; curve: Theme.curveHover } }
        }

        FocusRing { target: btn; radius: width / 2 }

        Icon {
            anchors.centerIn: parent
            name: btn.icon
            size: btn.iconSize
            spinning: btn.spinning
            color: btn.active ? btn.activeColor : btn.color
        }
    }

    MouseArea {
        id: area
        anchors.fill: parent
        anchors.margins: -btn.reach
        hoverEnabled: true
        enabled: btn.enabled
        onClicked: btn.clicked()
    }

    TipLoader {
        want: area.containsMouse && !area.pressed && btn.tip.length > 0
        cut: area.pressed
        hint: btn.tip
    }
}
