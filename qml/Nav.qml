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
    property string audioTab: "eq"

    property var renaming: null
    property var editing: null
    property var renamingFolder: null
    property var finder: null
    property string settingsSection: ""
    property var carried: null
    property var toast: null
    property var host: null
    property var restoring: null
    property real floor: 0
    // Keyboard focus draws its ring only when focus arrives with no pointer press in the last `pointerFocus` ms (the browsers' focus-visible rule); a key handler that moves focus calls keyboard() first.
    property bool pointing: false
    property double pointedAt: 0
    readonly property int pointerFocus: 500
    function pointed(down) { pointing = down; pointedAt = Date.now() }
    function keyboard() { pointedAt = 0 }
    function byKeyboard() { return !pointing && Date.now() - pointedAt > pointerFocus }
    signal arrived()
    readonly property bool canBack: stack.length > 0
    readonly property bool canForward: ahead.length > 0

    function here() { return { view: view, params: params, state: host ? host.capture() : null } }

    function land(name, args) {
        query = name === "search" ? (args.q || "") : ""
        view = name
        params = args
        arrived()
    }

    function show(entry) {
        restoring = entry.state || null
        land(entry.view, entry.params)
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

    function replace(name, args) { land(name, args || ({})) }

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
    function edit(id) {
        playlist(id)
        editing = id
    }
    function search(q) {
        leaveOverlays()
        query = q
        if (view === "search") params = { q: q }
        else go("search", { q: q })
    }
}
