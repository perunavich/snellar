import QtQuick
import Snellar

Item {
    id: bar
    property ViewHost host
    readonly property var page: host ? host.current : null
    readonly property var flick: host && page ? host.scroller(page) : null
    readonly property var head: page && page.barHead && page.barHead.visible ? page.barHead : null
    readonly property string title: head && page.barTitle ? page.barTitle : ""
    readonly property Item backButton: back
    readonly property real side: arrows.x + arrows.width + Theme.spaceLg
    readonly property color dim: Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, Theme.text.a * Theme.disabledOpacity)
    signal doubleTapped()
    readonly property real solid: {
        if (!flick || flick.contentY === undefined) return 0
        if (flick.mapToItem(bar, 0, 0).y >= height) return 0
        const past = head ? height + Theme.navBar - head.mapToItem(bar, 0, head.height).y : flick.contentY - flick.originY
        return Math.max(0, Math.min(1, past / Theme.navBar))
    }
    readonly property real floorSolid: {
        if (!flick || flick.contentY === undefined || Nav.floor <= 0) return 0
        const under = flick.originY + flick.contentHeight - flick.contentY - (flick.height - Nav.floor)
        return Math.max(0, Math.min(1, under / Theme.navBar))
    }

    height: Theme.navBar

    BarShade {
        anchors.fill: parent
        opacity: bar.solid
    }

    MouseArea {
        anchors.fill: parent
        onPressed: bar.forceActiveFocus()
    }

    WindowDrag {
        anchors { left: parent.left; right: parent.right; top: parent.top }
        height: Theme.spaceXl
        visible: Theme.frameless
        onDoubleTapped: bar.doubleTapped()
    }

    Row {
        id: arrows
        x: Theme.spaceLg
        y: Theme.spaceLg
        spacing: Theme.spaceXs

        PopButton {
            id: back
            icon: "back"
            tip: qsTr("Back")
            size: Theme.navButton
            iconSize: Theme.iconSm
            color: enabled ? Theme.text : bar.dim
            rest: Theme.navFill
            hover: Theme.navFillHover
            enabled: Nav.canBack
            onClicked: Nav.back()
        }
        PopButton {
            icon: "forward"
            tip: qsTr("Forward")
            size: Theme.navButton
            iconSize: Theme.iconSm
            color: enabled ? Theme.text : bar.dim
            rest: Theme.navFill
            hover: Theme.navFillHover
            enabled: Nav.canForward
            onClicked: Nav.forward()
        }
    }

    Text {
        anchors { left: arrows.right; right: parent.right; leftMargin: Theme.spaceLg; rightMargin: Theme.spaceLg; verticalCenter: arrows.verticalCenter }
        text: bar.title
        opacity: bar.solid
        visible: text.length > 0
        color: Theme.text
        font.family: Theme.font
        font.pixelSize: Theme.fsTitle2
        font.weight: Font.Bold
        elide: Text.ElideRight
    }
}
