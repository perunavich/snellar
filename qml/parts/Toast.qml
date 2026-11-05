import QtQuick
import Snellar

Rectangle {
    id: toast
    property string text
    property string actionText
    property var action: null
    readonly property real room: parent ? parent.width - 2 * Theme.gap : Theme.toastText
    width: Theme.snap(content.width + 2 * Theme.pillPadding)
    height: Math.max(Theme.controlLg, Theme.snap(content.height + 2 * Theme.spaceSm))
    radius: Math.min(height / 2, Theme.rLg)
    color: Theme.toastFill
    border.width: 1
    border.color: Theme.hairline
    opacity: 0
    scale: 0.92
    visible: opacity > 0

    Row {
        id: content
        anchors.centerIn: parent
        spacing: Theme.spaceLg

        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: toast.text
            color: Theme.text
            font.pixelSize: Theme.fsBody
            width: Math.min(implicitWidth, Theme.toastText, toast.room - 2 * Theme.pillPadding - (toast.action ? action.implicitWidth + content.spacing : 0))
            wrapMode: Text.WordWrap
        }
        Link {
            id: action
            anchors.verticalCenter: parent.verticalCenter
            visible: !!toast.action
            text: toast.actionText
            color: Theme.accent
            font.pixelSize: Theme.fsBody
            font.weight: Font.DemiBold
            onClicked: {
                const run = toast.action
                toast.action = null
                toast.state = ""
                run()
            }
        }
    }

    states: State {
        name: "shown"
        PropertyChanges { toast.opacity: 1; toast.scale: 1 }
    }
    transitions: Transition {
        NumberAnimation { properties: "opacity,scale"; duration: Theme.m3; easing.type: Easing.BezierSpline; easing.bezierCurve: Theme.curveStandard }
    }

    Timer {
        id: hide
        interval: Theme.toastTime
        onTriggered: { toast.state = ""; toast.action = null }
    }

    function show(text, actionText, action) {
        toast.text = text
        toast.actionText = actionText || ""
        toast.action = action || null
        toast.state = "shown"
        hide.interval = Math.max(action ? Theme.undoTime : Theme.toastTime, Math.min(Theme.toastLongest, text.length * Theme.toastPerChar))
        hide.restart()
    }

    Connections {
        target: Api
        function onToast(text) { toast.show(text) }
        function onOffer(text, actionText, action) { toast.show(text, actionText, action) }
    }
}
