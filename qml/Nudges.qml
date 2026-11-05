pragma Singleton
import QtQuick
import Snellar

QtObject {
    id: nudges

    property var current: null
    property var pending: []
    property bool paused: false
    property bool typing: false
    property string draft: ""
    property real remaining: 0

    function show(spec) {
        if (spec.group) pending = pending.filter(s => s.group !== spec.group)
        if (spec.front) {
            if (current && !(spec.group && current.group === spec.group)) pending = [current].concat(pending)
            open(spec)
            return
        }
        const queued = spec.id === (current && current.id) || pending.some(s => s.id === spec.id)
        if (queued) return
        if (current) pending = pending.concat([spec])
        else open(spec)
    }

    function open(spec) {
        remaining = spec.timeout || 0
        current = spec
    }

    function close() {
        const spec = current
        if (!spec) return
        current = null
        if (pending.length) {
            const next = pending[0]
            pending = pending.slice(1)
            open(next)
        }
    }

    function dismiss() { close() }

    function submit(text) {
        const spec = current
        if (!spec || !spec.field) return
        if (text.trim() && spec.field.run) spec.field.run(text.trim())
        close()
    }

    function choose(action) {
        if (action.run) action.run(draft.trim())
        close()
    }

    property Timer ticker: Timer {
        interval: Theme.nudgeTick
        repeat: true
        running: !!nudges.current && nudges.current.timeout > 0 && !nudges.paused && !nudges.typing
        onTriggered: {
            nudges.remaining -= Theme.nudgeTick / 1000
            if (nudges.remaining <= 0) nudges.close()
        }
    }
}
