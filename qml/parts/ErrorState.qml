import QtQuick
import Snellar

Column {
    id: state
    required property Fetch fetch
    property bool compact: !!fetch.data && fetch.own
    readonly property bool failed: fetch.code.length > 0
    visible: failed || (fetch.waiting && !compact)

    StaleNote { width: parent.width; fetch: state.fetch; visible: state.compact }
    EmptyState {
        width: parent.width
        visible: !state.compact
        icon: state.fetch.waiting ? "refresh" : (state.fetch.offline && state.fetch.failure !== "net.youtube_blocked" ? "offline" : "alert")
        text: state.fetch.waiting ? qsTr("Still waiting for an answer") : Errors.text(state.fetch.code)
        detail: !state.failed || !(state.fetch.offline || state.fetch.retrying) ? "" : state.fetch.offline ? qsTr("Snellar will try again when you're back online") : qsTr("Snellar will try again soon")
        actionText: state.fetch.waiting ? qsTr("Cancel") : qsTr("Try again")
        actionIcon: state.fetch.waiting ? "" : "refresh"
        onActed: state.fetch.waiting ? state.fetch.cancel() : state.fetch.reload()
    }
}
