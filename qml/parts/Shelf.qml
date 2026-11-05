import QtQuick
import Snellar

Item {
    id: shelf
    property string title
    property var items: []
    property Component delegate
    property int itemHeight: Theme.cardCover + Theme.cardText
    property string moreText: ""
    readonly property bool arrowsShown: list.contentWidth > list.width
    signal more()

    implicitHeight: head.height + itemHeight + Theme.spaceMd
    visible: items.length > 0

    Text {
        id: head
        text: shelf.title
        color: Theme.text
        font.family: Theme.font
        font.pixelSize: Theme.fsTitle1
        font.weight: Font.DemiBold
        height: Theme.controlMd
        verticalAlignment: Text.AlignVCenter
    }

    Row {
        anchors { right: parent.right; verticalCenter: head.verticalCenter }
        spacing: Theme.spaceXs

        Link {
            anchors.verticalCenter: parent.verticalCenter
            rightPadding: shelf.arrowsShown ? Theme.spaceSm : 0
            visible: shelf.moreText.length > 0
            text: shelf.moreText
            onClicked: shelf.more()
        }
        IconButton {
            visible: shelf.arrowsShown
            icon: "back"
            tip: qsTr("Scroll left")
            size: Theme.controlSm
            iconSize: Theme.iconSm
            filledBg: true
            enabled: list.contentX > 0
            opacity: enabled ? 1 : Theme.disabledOpacity
            onClicked: shelf.scroll(-1)
        }
        IconButton {
            visible: shelf.arrowsShown
            icon: "forward"
            tip: qsTr("Scroll right")
            size: Theme.controlSm
            iconSize: Theme.iconSm
            filledBg: true
            enabled: list.contentX < list.contentWidth - list.width - 1
            opacity: enabled ? 1 : Theme.disabledOpacity
            onClicked: shelf.scroll(1)
        }
    }

    function scroll(dir) {
        const to = Math.max(0, Math.min(list.contentWidth - list.width, list.contentX + dir * list.width * 0.8))
        slide.to = to
        slide.restart()
    }

    NumberAnimation {
        id: slide
        target: list
        property: "contentX"
        duration: Theme.emphasized
        easing.type: Easing.BezierSpline
        easing.bezierCurve: Theme.curveStandard
    }

    ListView {
        id: list
        anchors { left: parent.left; right: parent.right; top: head.bottom; topMargin: Theme.spaceSm }
        height: shelf.itemHeight
        orientation: ListView.Horizontal
        spacing: Theme.spaceLg
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        model: shelf.items
        delegate: shelf.delegate
    }
}
