pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import Snellar

Item {
    id: backdrop
    property string part
    property bool moving: false
    property DaypartSky front: one

    function show(next) {
        const back = front === one ? two : one
        back.part = next
        back.visible = true
        back.opacity = 0
        back.z = 1
        front.z = 0
        front = back
        fade.restart()
    }

    onPartChanged: if (front.part !== part) show(part)
    Component.onCompleted: one.part = part

    layer.enabled: true
    layer.effect: MultiEffect { maskEnabled: true; maskSource: shape }

    Rectangle {
        id: shape
        anchors.fill: parent
        radius: Theme.rCard
        visible: false
        layer.enabled: true
    }

    DaypartSky {
        id: one
        objectName: "daypartSky"
        anchors.fill: parent
        moving: backdrop.moving && visible
    }

    DaypartSky {
        id: two
        objectName: "daypartSky"
        anchors.fill: parent
        visible: false
        moving: backdrop.moving && visible
    }

    NumberAnimation {
        id: fade
        target: backdrop.front
        property: "opacity"
        from: 0
        to: 1
        duration: Theme.ambientFade
        easing.type: Easing.OutCubic
        onFinished: (backdrop.front === one ? two : one).visible = false
    }
}
