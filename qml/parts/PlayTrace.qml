import QtQuick
import Snellar

Column {
    id: trace
    readonly property var failure: Api.failure
    readonly property bool failed: !!failure && !!Api.current && failure.id === Api.current.id && !Api.loading
    property bool waited: false
    spacing: Theme.spaceSm
    visible: failed || (waited && Api.loading)

    Timer {
        interval: Theme.waitPatience
        running: Api.loading
        onRunningChanged: if (running) trace.waited = false
        onTriggered: trace.waited = true
    }

    Row {
        width: parent.width
        spacing: Theme.spaceSm
        visible: trace.waited && Api.loading

        Text {
            anchors.verticalCenter: parent.verticalCenter
            width: parent.width - cancel.implicitWidth - parent.spacing
            text: qsTr("Still trying to start this song")
            color: Theme.text2
            font.pixelSize: Theme.fsCaption
            wrapMode: Text.WordWrap
        }
        Link {
            id: cancel
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Cancel")
            color: Theme.accent
            font.pixelSize: Theme.fsCaption
            font.weight: Font.DemiBold
            onClicked: Api.call("cancelStart")
        }
    }

    Text {
        width: parent.width
        visible: trace.failed && trace.failure.stopped
        text: qsTr("Playback stopped: 3 tracks in a row could not play")
        color: Theme.text
        font.pixelSize: Theme.fsBody
        font.weight: Font.DemiBold
        wrapMode: Text.WordWrap
    }

    Row {
        width: parent.width
        spacing: Theme.spaceSm
        visible: trace.failed

        Icon { y: Theme.spaceXs / 2; name: "alert"; size: Theme.iconSm; color: Theme.red }
        Text {
            width: parent.width - Theme.iconSm - parent.spacing
            text: trace.failure ? Errors.text(trace.failure.code) : ""
            color: Theme.text2
            font.pixelSize: Theme.fsCaption
            wrapMode: Text.WordWrap
        }
    }

    Text {
        width: parent.width
        visible: trace.failed && trace.failure.retrying
        text: qsTr("Snellar will try again soon")
        color: Theme.text3
        font.pixelSize: Theme.fsCaption
    }

    Row {
        spacing: Theme.spaceMd
        visible: trace.failed

        PillButton {
            anchors.verticalCenter: parent.verticalCenter
            height: Theme.controlMd
            text: qsTr("Try again")
            icon: "refresh"
            onClicked: Api.call("jump", { index: Api.cursor })
        }
        Link {
            anchors.verticalCenter: parent.verticalCenter
            visible: Api.cursor + 1 < Api.queue.length
            text: trace.failed && trace.failure.stopped ? qsTr("Continue") : qsTr("Skip")
            color: Theme.accent
            font.pixelSize: Theme.fsBody
            font.weight: Font.DemiBold
            onClicked: Api.next()
        }
    }
}
