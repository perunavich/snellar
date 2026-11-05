pragma Singleton
import QtQuick
import Snellar

QtObject {
    id: imports

    property bool running: false
    property string stage: ""
    property int done: 0
    property int total: 0
    property int matched: 0
    property var result: null
    property string error: ""
    property int run: 0

    function start(args) {
        const mine = ++run
        running = true
        result = null
        error = ""
        stage = "reading"
        done = total = matched = 0
        Api.call("importPlaylist", args, (v, e, code) => {
            if (mine !== run) return
            running = false
            if (e) {
                error = code || "internal"
                return
            }
            result = v
            if (v.playlist !== null) Api.libraryChanged()
        })
    }

    function cancel() {
        if (!running) return
        run++
        running = false
        Api.call("importCancel")
    }

    function clear() {
        result = null
        error = ""
    }

    function progress(p) {
        if (!running || !p) return
        stage = p.stage
        done = p.done
        total = p.total
        matched = p.matched
    }

    function lines() {
        return (result ? result.unmatched : []).map(t => [t.artists.join(", "), t.title].filter(x => x).join(" - ")).join("\n")
    }

    property Connections fromCore: Connections {
        target: Core
        function onImportProgress(json) { imports.progress(Api.parse(json, null)) }
    }
    property Connections fromMock: Connections {
        target: Api.mp
        ignoreUnknownSignals: true
        function onImportProgress(p) { imports.progress(p) }
    }
}
