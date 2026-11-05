import QtQuick
import Snellar

Column {
    id: state
    required property Fetch fetch
    readonly property bool failed: fetch.code.length > 0
    visible: failed || fetch.waiting
    spacing: Theme.spaceMd

    Icon {
        anchors.horizontalCenter: parent.horizontalCenter
        name: state.fetch.waiting ? "refresh" : (state.fetch.offline ? "offline" : "alert")
        size: Theme.iconXl
        color: Theme.text3
    }
    Text {
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        text: state.fetch.waiting ? qsTr("Still waiting for an answer") : Errors.text(state.fetch.code)
        color: Theme.text
        font.pixelSize: Theme.fsHeadline
        font.weight: Font.DemiBold
    }
    Text {
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.WordWrap
        visible: state.failed && (state.fetch.offline || state.fetch.retrying)
        text: state.fetch.offline ? qsTr("Snellar will try again when the connection is back") : qsTr("Snellar will try again soon")
        color: Theme.text2
        font.pixelSize: Theme.fsBody
    }
    PillButton {
        anchors.horizontalCenter: parent.horizontalCenter
        text: state.fetch.waiting ? qsTr("Cancel") : qsTr("Try again")
        icon: state.fetch.waiting ? "" : "refresh"
        onClicked: state.fetch.waiting ? state.fetch.cancel() : state.fetch.reload()
    }
}
