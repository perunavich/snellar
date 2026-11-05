import QtQuick
import QtQuick.Effects
import Snellar

Item {
    id: amb
    property string source
    property bool flip: false
    property real strength: Theme.coverBackdrop
    readonly property real shrink: 12

    onSourceChanged: {
        flip = !flip
        const img = flip ? a : b
        img.source = source
    }

    Item {
        width: amb.width / amb.shrink
        height: amb.height / amb.shrink
        scale: amb.shrink
        transformOrigin: Item.TopLeft

        Image {
            id: a
            anchors.fill: parent
            anchors.margins: -parent.width * 0.03
            sourceSize.width: 120
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            visible: false
        }
        Image {
            id: b
            anchors.fill: a
            sourceSize.width: 120
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            visible: false
        }

        MultiEffect {
            anchors.fill: a
            source: a
            blurEnabled: true
            blur: 1.0
            blurMax: 8
            saturation: 0.4
            opacity: amb.flip && a.status === Image.Ready ? amb.strength : 0
            Behavior on opacity { NumberAnimation { duration: Theme.ambientFade; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard } }
        }
        MultiEffect {
            anchors.fill: b
            source: b
            blurEnabled: true
            blur: 1.0
            blurMax: 8
            saturation: 0.4
            opacity: !amb.flip && b.status === Image.Ready ? amb.strength : 0
            Behavior on opacity { NumberAnimation { duration: Theme.ambientFade; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard } }
        }
    }

    Rectangle {
        anchors.fill: parent
        gradient: Gradient {
            GradientStop { position: 0; color: Theme.ambientTop }
            GradientStop { position: 1; color: Theme.ambientBottom }
        }
    }
}
