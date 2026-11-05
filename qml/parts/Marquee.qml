import QtQuick
import Snellar

Item {
    id: marquee
    property alias text: label.text
    property alias font: label.font
    property alias color: label.color
    readonly property real overflow: Math.max(0, label.implicitWidth - width)
    readonly property bool truncated: label.truncated
    readonly property bool scrolling: overflow > 0 && visible && Window.active
    property bool moving: false
    implicitHeight: label.implicitHeight
    clip: true

    Text {
        id: label
        width: marquee.moving ? implicitWidth : marquee.width
        elide: marquee.moving ? Text.ElideNone : Text.ElideRight
        wrapMode: Text.NoWrap

        SequentialAnimation on x {
            running: marquee.scrolling
            loops: Animation.Infinite
            PauseAnimation { duration: Theme.marqueePause }
            PropertyAction { target: marquee; property: "moving"; value: true }
            NumberAnimation { to: -marquee.overflow; duration: marquee.overflow * Theme.marqueeMsPerPx; easing.type: Easing.InOutSine }
            PauseAnimation { duration: Theme.marqueePause }
            NumberAnimation { to: 0; duration: marquee.overflow * Theme.marqueeMsPerPx; easing.type: Easing.InOutSine }
            PropertyAction { target: marquee; property: "moving"; value: false }
        }
    }

    onScrollingChanged: if (!scrolling) {
        moving = false
        label.x = 0
    }
}
