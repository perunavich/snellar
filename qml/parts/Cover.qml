import QtQuick
import Snellar

Item {
    id: cover
    property string url
    property real radius: Theme.rMd
    property int px: 256
    property alias status: img.status

    Rectangle {
        anchors.fill: parent
        radius: cover.radius
        color: Theme.fillRaised
        visible: img.status !== Image.Ready

        Icon {
            anchors.centerIn: parent
            name: "music"
            size: Math.min(parent.width, parent.height) * 0.36
            color: Theme.text3
        }
    }

    Image {
        id: img
        anchors.fill: parent
        source: Theme.art(cover.url, cover.px)
        sourceSize.width: cover.px
        sourceSize.height: cover.px
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        visible: false
    }

    ShaderEffect {
        anchors.fill: parent
        property Item src: img
        property real wPx: width
        property real hPx: height
        property real radiusPx: cover.radius
        fragmentShader: "../../shaders/round.frag.qsb"
        opacity: img.status === Image.Ready ? 1 : 0
        Behavior on opacity { NumberAnimation { duration: Theme.m3 } }
    }
}
