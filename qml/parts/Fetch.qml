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
    property bool seeded: false
    property bool own: false
    property var raw

    onDataChanged: raw = undefined

    function known() {
        if (raw === undefined) raw = JSON.stringify(data)
        return raw
    }

    function seed(over) {
        if ((data && !over) || !ready || !method) return false
        const text = {}
        const kept = Api.peek(method, args, text)
        if (kept === undefined) return false
        data = kept
        raw = text.raw
        seeded = true
        own = true
        return true
    }

    function reload(quiet) {
        if (!ready || !method) return
        later.stop()
        const mine = ++fetch.serial
        const kept = fetch.seed() || fetch.seeded
        fetch.seeded = false
        if (!quiet) {
            fetch.tries = 0
            fetch.retrying = false
            fetch.code = ""
            if (!kept) {
                fetch.loading = true
                patience.restart()
            }
        }
        Api.call(method, args, (v, e, code, after, text) => {
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
                fetch.own = true
                const reply = text !== undefined ? text : JSON.stringify(v)
                if (!fetch.data || reply !== fetch.known()) {
                    fetch.data = v
                    fetch.raw = reply
                }
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
        if (!seed(true)) {
            own = false
            if (!keepStale) data = null
        }
        Qt.callLater(reload)
    }
    onReadyChanged: {
        seed()
        Qt.callLater(reload)
    }
    Component.onCompleted: {
        seed()
        Qt.callLater(reload)
    }
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
