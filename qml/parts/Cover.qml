pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Effects
import Snellar

Item {
    id: cover
    property string url
    property real radius: Theme.rMd
    property int px: 256
    property int decode: px
    property bool standIn: false
    property bool note: true
    property alias status: img.status
    readonly property bool standingIn: standIn && thumb.status === Image.Ready && mask.opacity < 1
    property bool swapping: false

    Rectangle {
        anchors.fill: parent
        radius: cover.radius
        color: Theme.fillRaised
        visible: img.status !== Image.Ready && !cover.standingIn

        Icon {
            anchors.centerIn: parent
            visible: cover.note
            name: "music"
            size: Theme.iconFit(Math.min(parent.width, parent.height) * Theme.coverNoteShare)
            color: Theme.text3
        }
    }

    Image {
        id: thumb
        anchors.fill: parent
        source: cover.standIn && (img.status !== Image.Ready || fadeIn.running) ? Theme.art(cover.url, Theme.coverRowPx) : ""
        sourceSize.width: Theme.coverRowPx
        sourceSize.height: Theme.coverRowPx
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        visible: false
    }

    Loader {
        anchors.fill: parent
        active: cover.standIn
        sourceComponent: Item {
            visible: cover.standingIn
            layer.enabled: visible
            layer.samplerName: "src"
            layer.effect: RoundMask { radius: cover.radius }

            MultiEffect {
                anchors.fill: parent
                anchors.margins: -blurMax
                source: thumb
                blurEnabled: true
                blur: 1
                blurMax: Math.max(4, Math.round(cover.width * Theme.standInBlur))
            }
        }
    }

    Image {
        id: img
        anchors.fill: parent
        source: Theme.art(cover.url, cover.px)
        sourceSize.width: Math.min(cover.px, cover.decode)
        sourceSize.height: Math.min(cover.px, cover.decode)
        fillMode: Image.PreserveAspectCrop
        asynchronous: true
        visible: false
        onSourceChanged: {
            cover.swapping = true
            settle.restart()
        }
        onStatusChanged: {
            fadeIn.stop()
            if (status !== Image.Ready) mask.opacity = 0
            else if (cover.swapping) mask.opacity = 1
            else fadeIn.restart()
        }
        Component.onCompleted: if (status === Image.Ready) mask.opacity = 1
    }

    RoundMask {
        id: mask
        anchors.fill: parent
        property Item src: img
        picture: Qt.size(img.implicitWidth, img.implicitHeight)
        radius: cover.radius
        opacity: 0
    }

    Timer {
        id: settle
        interval: 0
        onTriggered: cover.swapping = false
    }

    NumberAnimation {
        id: fadeIn
        target: mask
        property: "opacity"
        from: 0
        to: 1
        duration: Theme.coverFade
    }
}
