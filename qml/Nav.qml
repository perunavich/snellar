pragma Singleton
import QtQuick

QtObject {
    property string view: "discover"
    property var params: ({})
    property var stack: []
    property var ahead: []
    property string query: ""

    property bool nowPlaying: false
    property bool audio: false
    property bool keys: false
    property bool picking: false
    property string tab: "next"

    property var renaming: null
    property var renamingFolder: null
    property var finder: null
    property string settingsSection: ""
    property var carried: null
    property var host: null
    property var restoring: null
    readonly property bool canBack: stack.length > 0
    readonly property bool canForward: ahead.length > 0

    function here() { return { view: view, params: params, state: host ? host.capture() : null } }

    function show(entry) {
        restoring = entry.state || null
        view = entry.view
        params = entry.params
    }

    function leaveOverlays() {
        nowPlaying = false
        audio = false
    }

    function go(name, args) {
        leaveOverlays()
        if (name === view && JSON.stringify(args || {}) === JSON.stringify(params)) return
        stack = stack.concat([here()])
        ahead = []
        show({ view: name, params: args || ({}) })
    }

    function replace(name, args) {
        view = name
        params = args || ({})
    }

    function root(name) {
        leaveOverlays()
        stack = []
        ahead = []
        show({ view: name, params: ({}) })
    }

    function back() {
        if (audio || nowPlaying) {
            if (audio) audio = false
            else nowPlaying = false
            return
        }
        if (!stack.length) return
        const top = stack[stack.length - 1]
        stack = stack.slice(0, -1)
        ahead = ahead.concat([here()])
        show(top)
    }

    function forward() {
        if (!ahead.length) return
        leaveOverlays()
        const next = ahead[ahead.length - 1]
        ahead = ahead.slice(0, -1)
        stack = stack.concat([here()])
        show(next)
    }

    function album(id) { go("album", { id: id }) }
    function artist(id) { go("artist", { id: id }) }
    function discography(id, kind) { go("discography", { id: id, kind: kind }) }
    function playlist(id) { go("playlist", { id: id }) }
    function remote(id) { go("album", { id: id, playlist: true }) }
    function rename(id) {
        playlist(id)
        renaming = id
    }
    function search(q) {
        leaveOverlays()
        query = q
        if (view === "search") params = { q: q }
        else go("search", { q: q })
    }
}
