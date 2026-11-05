import QtQuick
import Snellar

QtObject {
    id: fetch
    property string method
    property var args: ({})
    property bool ready: true
    property bool live: false
    property bool keepStale: false
    property var data: null
    property bool loading: false
    property string code: ""
    readonly property string note: data && data.problem ? data.problem : ""
    readonly property string failure: code || note
    readonly property bool offline: Errors.unreachable(failure)
    property bool retrying: false
    property bool waiting: false
    property int serial: 0
    property int tries: 0

    function reload(quiet) {
        if (!ready || !method) return
        later.stop()
        const mine = ++fetch.serial
        if (!quiet) {
            fetch.tries = 0
            fetch.retrying = false
            fetch.loading = true
            fetch.code = ""
            patience.restart()
        }
        Api.call(method, args, (v, e, code, after) => {
            if (!fetch || mine !== fetch.serial) return
            fetch.loading = false
            fetch.waiting = false
            patience.stop()
            if (e) {
                fetch.code = code || "internal"
                Api.strand(fetch, fetch.offline)
                fetch.plan(after)
            } else {
                const back = Api.stranded.indexOf(fetch) >= 0
                fetch.code = ""
                fetch.data = v
                Api.strand(fetch, fetch.offline)
                if (back) Api.retry()
                fetch.plan(v && v.retryAfter)
            }
        })
    }

    function plan(after) {
        retrying = Errors.passing(failure) && tries < Theme.retryLimit
        if (!retrying) {
            if (!failure) tries = 0
            return
        }
        const wait = Api.retryBase * Math.pow(2, tries++)
        later.interval = Math.max(wait, (after || 0) * 1000, Api.hold(after) - Date.now())
        later.restart()
    }

    function cancel() {
        fetch.serial++
        fetch.loading = false
        fetch.waiting = false
        patience.stop()
        later.stop()
        fetch.retrying = false
        fetch.code = "fetch.cancelled"
    }

    property Timer later: Timer { onTriggered: fetch.reload(true) }
    property Timer patience: Timer {
        interval: Theme.waitPatience
        onTriggered: {
            if (!fetch.loading) return
            fetch.loading = false
            fetch.waiting = true
        }
    }

    onArgsChanged: {
        if (!keepStale) data = null
        Qt.callLater(reload)
    }
    onReadyChanged: Qt.callLater(reload)
    Component.onCompleted: Qt.callLater(reload)
    Component.onDestruction: Api.strand(fetch, false)

    property Connections link: Connections {
        target: Api
        enabled: fetch.live
        function onLibraryChanged() { fetch.reload() }
    }

    property Connections again: Connections {
        target: Api
        enabled: fetch.offline
        function onRetry() { fetch.reload(true) }
    }
}
