import QtQuick
import Snellar

Item {
    id: root
    property bool on: false
    property bool animated: true
    readonly property real knobValue: knob.value
    readonly property real stretchValue: stretch.value
    readonly property bool pressed: area.pressed
    signal toggled()

    width: Theme.switchWidth
    height: Theme.switchHeight
    opacity: enabled ? 1 : Theme.disabledOpacity
    onOnChanged: if (!animated) knob.snap(on ? 1 : 0)

    SpringValue {
        id: knob
        reducible: true
        zeta: Motion.controlDamping
        response: Motion.controlResponse
        target: root.on ? 1 : 0
    }

    SpringValue {
        id: stretch
        reducible: true
        zeta: area.pressed ? Motion.trackingDamping : Motion.pressDampingFor(root.width, root.height)
        response: area.pressed ? Motion.trackingResponseFor(root.width, root.height) : Motion.pressResponseFor(root.width, root.height)
        target: area.pressed ? Motion.switchStretch : 0
    }

    Rectangle {
        readonly property real k: Math.max(0, Math.min(1, knob.value))
        anchors.fill: parent
        radius: height / 2
        color: Motion.blend(Theme.fillStrong, Theme.accent, k)

        Rectangle {
            readonly property real size: parent.height - 2 * Theme.segmentInset
            width: size + stretch.value
            height: size
            radius: height / 2
            color: Motion.blend(Theme.white, Theme.accentInk, parent.k)
            y: Theme.segmentInset
            x: Theme.segmentInset + (parent.width - 2 * Theme.segmentInset - width) * knob.value
        }

        Rectangle {
            objectName: "wash"
            anchors.fill: parent
            radius: height / 2
            color: Theme.text
            opacity: area.pressed ? Theme.fillPress.a : area.containsMouse ? Theme.fillHover.a : 0
            Behavior on opacity { NumberAnimation { duration: Theme.hover } }
        }
    }

    FocusRing { target: root; radius: height / 2 }

    MouseArea {
        id: area
        anchors.fill: parent
        enabled: root.enabled
        hoverEnabled: true
        onClicked: root.toggled()
    }
}
